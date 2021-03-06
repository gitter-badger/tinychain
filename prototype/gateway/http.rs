use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures::future::{self, Future, TryFutureExt};
use futures::stream::{self, Stream, StreamExt, TryStreamExt};
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, StatusCode, Uri};
use log::debug;
use serde::de::DeserializeOwned;
use tokio::time::timeout;

use crate::auth::Token;
use crate::class::State;
use crate::error;
use crate::request::Request;
use crate::scalar::value::link::*;
use crate::scalar::{Id, Scalar, Value};
use crate::stream::{JsonListStream, StreamBuffer};
use crate::transaction::Txn;
use crate::{TCResult, TCStream};

use super::Gateway;

const CONTENT_TYPE: &str = "application/json; charset=utf-8";
const ERR_DECODE: &str = "(unable to decode error message)";

pub struct Client {
    client: hyper::Client<hyper::client::HttpConnector, Body>,
    response_limit: usize,
}

impl Client {
    pub fn new(ttl: Duration, response_limit: usize) -> Client {
        let client = hyper::Client::builder()
            .pool_idle_timeout(ttl)
            .http2_only(true)
            .build_http();

        Client {
            client,
            response_limit,
        }
    }

    pub async fn get(
        &self,
        request: &Request,
        txn: &Txn,
        link: &Link,
        key: &Value,
    ) -> TCResult<Scalar> {
        if request.auth().is_some() {
            return Err(error::not_implemented("Authorization"));
        }

        let host = link
            .host()
            .as_ref()
            .ok_or_else(|| error::bad_request("No host to resolve", &link))?;

        let host = if let Some(port) = host.port() {
            format!("{}:{}", host.address(), port)
        } else {
            host.address().to_string()
        };

        let path_and_query = if key == &Value::None {
            link.path().to_string()
        } else {
            let key: String = serde_json::to_string(key).map_err(error::TCError::from)?;
            format!("{}?key={}&txn_id={}", link.path(), key, txn.id())
        };

        let uri = format!("http://{}{}", host, path_and_query)
            .parse()
            .map_err(|err| error::bad_request("Unable to encode link URI", err))?;

        match timeout(request.ttl(), self.client.get(uri)).await {
            Err(_) => Err(error::bad_request("Timed out awaiting", link)),
            Ok(result) => match result {
                Err(cause) => Err(error::transport(cause)),
                Ok(response) if response.status() != 200 => {
                    let status = response.status().as_u16();
                    let msg = if let Ok(msg) = hyper::body::to_bytes(response).await {
                        if let Ok(msg) = String::from_utf8(msg.to_vec()) {
                            msg
                        } else {
                            ERR_DECODE.to_string()
                        }
                    } else {
                        ERR_DECODE.to_string()
                    };

                    Err(error::TCError::of(status.into(), msg))
                }
                Ok(mut response) => {
                    deserialize_body(response.body_mut(), self.response_limit).await
                }
            },
        }
    }

    pub async fn post<'a, S: Stream<Item = (Id, Scalar)> + Send + 'a>(
        &'a self,
        request: &'a Request,
        txn: &'a Txn,
        link: Link,
        data: S,
    ) -> TCResult<State>
    where
        S: 'static,
    {
        if request.auth().is_some() {
            return Err(error::not_implemented("Authorization"));
        }

        let host = link
            .host()
            .as_ref()
            .ok_or_else(|| error::bad_request("No host to resolve", &link))?;

        let uri = Uri::builder()
            .scheme(host.protocol().to_string().as_str())
            .authority(host.authority().as_str())
            .path_and_query(format!("{}?txn_id={}", link.path(), txn.id()).as_str())
            .build()
            .map_err(error::internal)?;

        debug!("POST to {}", uri);

        let req = hyper::Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::wrap_stream(JsonListStream::from(data)))
            .map_err(error::internal)?;

        match timeout(request.ttl(), self.client.request(req)).await {
            Err(_) => Err(error::bad_request("The request timed out waiting on", link)),
            Ok(result) => match result {
                Err(cause) => Err(error::transport(cause)),
                Ok(response) if response.status() != 200 => {
                    let status = response.status().as_u16();
                    let msg = if let Ok(msg) = hyper::body::to_bytes(response).await {
                        if let Ok(msg) = String::from_utf8(msg.to_vec()) {
                            msg
                        } else {
                            ERR_DECODE.to_string()
                        }
                    } else {
                        ERR_DECODE.to_string()
                    };

                    Err(error::TCError::of(status.into(), msg))
                }
                Ok(_) => {
                    // TODO: deserialize response
                    Ok(().into())
                }
            },
        }
    }
}

pub struct Server {
    address: SocketAddr,
    request_limit: usize,
    request_ttl: Duration,
}

impl Server {
    pub fn new(address: SocketAddr, request_limit: usize, request_ttl: Duration) -> Server {
        Server {
            address,
            request_limit,
            request_ttl,
        }
    }

    async fn handle(
        self: Arc<Self>,
        gateway: Arc<Gateway>,
        http_request: hyper::Request<Body>,
    ) -> TCResult<(State, Txn)> {
        let token: Option<Token> = if let Some(header) = http_request.headers().get("Authorization")
        {
            let token = header
                .to_str()
                .map_err(|e| error::bad_request("Unable to parse Authorization header", e))?;

            Some(gateway.authenticate(token).await?)
        } else {
            None
        };

        let mut params = http_request
            .uri()
            .query()
            .map(|v| {
                debug!("param {}", v);
                url::form_urlencoded::parse(v.as_bytes())
                    .into_owned()
                    .collect()
            })
            .unwrap_or_else(HashMap::new);

        let txn_id = get_param(&mut params, "txn_id")?;

        let request = Request::new(self.request_ttl, token, txn_id);
        let result = timeout(
            request.ttl(),
            self.route(gateway, request, params, http_request),
        )
        .map_err(|e| error::timeout(e))
        .await?;

        result
    }

    async fn route(
        self: Arc<Self>,
        gateway: Arc<Gateway>,
        request: Request,
        mut params: HashMap<String, String>,
        mut http_request: hyper::Request<Body>,
    ) -> TCResult<(State, Txn)> {
        let uri = http_request.uri().clone();
        let path: TCPathBuf = uri.path().parse()?;
        let txn = gateway.transaction(&request).await?;

        let state = match http_request.method() {
            &Method::GET => {
                let id = get_param(&mut params, "key")?.unwrap_or_else(|| Value::None);
                gateway.get(&request, &txn, &path.into(), id).await
            }

            &Method::PUT => {
                debug!("PUT {}", path);
                let id = get_param(&mut params, "key")?
                    .ok_or_else(|| error::bad_request("Missing URI parameter", "'key'"))?;
                let value: Scalar =
                    deserialize_body(http_request.body_mut(), self.request_limit).await?;

                gateway
                    .put(&request, &txn, &path.into(), id, value.into())
                    .map_ok(State::from)
                    .await
            }

            &Method::POST => {
                debug!("POST {}", path);
                let request_body: Scalar =
                    deserialize_body(http_request.body_mut(), self.request_limit).await?;

                gateway
                    .post(&request, &txn, path.into(), request_body)
                    .await
            }

            &Method::DELETE => {
                let id = get_param(&mut params, "key")?.unwrap_or_else(|| Value::None);
                gateway
                    .delete(&request, &txn, &path.into(), id)
                    .map_ok(State::from)
                    .await
            }

            other => Err(error::method_not_allowed(format!(
                "Tinychain does not support {}",
                other
            ))),
        }?;

        Ok((state, txn))
    }
}

async fn deserialize_body<D: DeserializeOwned>(
    body: &mut hyper::Body,
    max_size: usize,
) -> TCResult<D> {
    let mut buffer = vec![];
    while let Some(chunk) = body.next().await {
        buffer.extend(chunk?.to_vec());

        if buffer.len() > max_size {
            return Err(error::too_large(max_size));
        }
    }

    let data = String::from_utf8(buffer)
        .map_err(|e| error::bad_request("Unable to parse request body", e))?;

    serde_json::from_str(&data).map_err(|e| {
        error::bad_request(
            &format!("Deserialization error \"{}\" when parsing", e),
            data,
        )
    })
}

async fn to_stream<'a>(state: State, txn: Txn) -> TCResult<TCStream<'a, TCResult<Bytes>>> {
    match state {
        State::Collection(collection) => {
            let buffer = StreamBuffer::new(collection, txn).await?;
            let json = JsonListStream::from(buffer.into_stream());
            let response = Box::pin(json.map_ok(Bytes::from).chain(stream_delimiter(b"\r\n")));
            Ok(response)
        }
        State::Scalar(scalar) => {
            let response = serde_json::to_string_pretty(&scalar)
                .map(|s| format!("{}\r\n", s))
                .map(Bytes::from)
                .map_err(error::TCError::from)?;

            let response: TCStream<'a, TCResult<Bytes>> =
                Box::pin(stream::once(future::ready(Ok(response))));

            Ok(response)
        }
        other => Err(error::not_implemented(format!(
            "Streaming serialization for {}",
            other
        ))),
    }
}

fn stream_delimiter<'a>(token: &[u8]) -> TCStream<'a, TCResult<Bytes>> {
    let token = Bytes::copy_from_slice(token);
    Box::pin(stream::once(future::ready(Ok(token))))
}

#[async_trait]
impl super::Server for Server {
    type Error = hyper::Error;

    async fn listen(self: Arc<Self>, gateway: Arc<Gateway>) -> Result<(), Self::Error> {
        hyper::Server::bind(&self.address)
            .serve(make_service_fn(|_conn| {
                let this = self.clone();
                let gateway = gateway.clone();
                async {
                    Ok::<_, Infallible>(service_fn(move |request| {
                        let method = request.method().clone();
                        let state = this.clone().handle(gateway.clone(), request);
                        encode_response(method, state)
                    }))
                }
            }))
            .await
    }
}

async fn encode_response(
    method: Method,
    result: impl Future<Output = TCResult<(State, Txn)>>,
) -> Result<hyper::Response<Body>, hyper::Error> {
    let success_code = if method == Method::PUT || method == Method::DELETE {
        StatusCode::NO_CONTENT // 204, no response content
    } else {
        StatusCode::OK // 200, content to follow
    };

    let mut response = match result.await {
        Err(cause) => transform_error(cause),
        Ok((state, txn)) => {
            let response = to_stream(state, txn).await.unwrap();
            let mut response = hyper::Response::new(Body::wrap_stream(response));
            *response.status_mut() = success_code;
            response
        }
    };

    response
        .headers_mut()
        .insert(hyper::header::CONTENT_TYPE, CONTENT_TYPE.parse().unwrap());

    Ok(response)
}

fn encode_query_string(data: Vec<(&str, &str)>) -> String {
    let mut query_string = url::form_urlencoded::Serializer::new(String::new());
    for (name, value) in data.into_iter() {
        query_string.append_pair(name, value);
    }
    query_string.finish()
}

fn get_param<T: DeserializeOwned>(
    params: &mut HashMap<String, String>,
    name: &str,
) -> TCResult<Option<T>> {
    if let Some(param) = params.remove(name) {
        let val: T = serde_json::from_str(&param).map_err(|e| {
            error::bad_request(&format!("Unable to parse URI parameter '{}'", name), e)
        })?;

        Ok(Some(val))
    } else {
        Ok(None)
    }
}

fn transform_error(err: error::TCError) -> hyper::Response<Body> {
    let mut response = hyper::Response::new(Body::from(format!("{}\r\n", err.message())));

    use error::ErrorType::*;
    *response.status_mut() = match err.reason() {
        BadRequest => StatusCode::BAD_REQUEST,
        Conflict => StatusCode::CONFLICT,
        Forbidden => StatusCode::FORBIDDEN,
        Internal => StatusCode::INTERNAL_SERVER_ERROR,
        MethodNotAllowed => StatusCode::METHOD_NOT_ALLOWED,
        NotFound => StatusCode::NOT_FOUND,
        NotImplemented => StatusCode::NOT_IMPLEMENTED,
        Timeout => StatusCode::REQUEST_TIMEOUT,
        TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
        Transport => StatusCode::from_u16(499).unwrap(), // custom status code
        Unauthorized => StatusCode::UNAUTHORIZED,
        Unknown => StatusCode::INTERNAL_SERVER_ERROR,
    };

    response
}
