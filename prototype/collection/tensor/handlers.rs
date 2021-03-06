use std::iter::FromIterator;

use async_trait::async_trait;

use crate::auth::{Scope, SCOPE_READ, SCOPE_WRITE};
use crate::class::{State, TCType};
use crate::collection::Collection;
use crate::error;
use crate::general::Map;
use crate::handler::*;
use crate::request::Request;
use crate::scalar::{label, MethodType, NumberType, PathSegment, Scalar, Value};
use crate::transaction::Txn;
use crate::{TCResult, TryCastInto};

use super::bounds::*;
use super::class::{Tensor, TensorInstance};
use super::{IntoView, TensorDualIO, TensorUnary};

struct AllHandler<'a, T: TensorInstance> {
    tensor: &'a T,
}

#[async_trait]
impl<'a, T: TensorInstance> Handler for AllHandler<'a, T> {
    fn subject(&self) -> TCType {
        self.tensor.class().into()
    }

    fn scope(&self) -> Option<Scope> {
        Some(SCOPE_READ.into())
    }

    async fn handle_get(self: Box<Self>, txn: &Txn, selector: Value) -> TCResult<State> {
        let all = if selector.is_none() {
            self.tensor.all(txn).await
        } else {
            let bounds =
                selector.try_cast_into(|v| error::bad_request("Invalid Tensor bounds", v))?;
            let slice = self.tensor.slice(bounds)?;
            slice.into_view().all(txn).await
        };

        all.map(Value::from).map(State::from)
    }
}

struct AnyHandler<'a, T: TensorInstance> {
    tensor: &'a T,
}

#[async_trait]
impl<'a, T: TensorInstance> Handler for AnyHandler<'a, T> {
    fn subject(&self) -> TCType {
        self.tensor.class().into()
    }

    fn scope(&self) -> Option<Scope> {
        Some(SCOPE_READ.into())
    }

    async fn handle_get(self: Box<Self>, txn: &Txn, selector: Value) -> TCResult<State> {
        let any = if selector.is_none() {
            self.tensor.any(txn).await
        } else {
            let bounds =
                selector.try_cast_into(|v| error::bad_request("Invalid Tensor bounds", v))?;
            let slice = self.tensor.slice(bounds)?;
            slice.into_view().any(txn).await
        };

        any.map(Value::from).map(State::from)
    }
}

struct GetHandler<'a, T: TensorInstance, R: Fn(&T, &Txn, Value) -> TCResult<State> + Send + Sync> {
    tensor: &'a T,
    read_fn: R,
}

#[async_trait]
impl<'a, T: TensorInstance, R: Fn(&T, &Txn, Value) -> TCResult<State> + Send + Sync> Handler
    for GetHandler<'a, T, R>
{
    fn subject(&self) -> TCType {
        self.tensor.class().into()
    }

    fn scope(&self) -> Option<Scope> {
        Some(SCOPE_READ.into())
    }

    async fn handle_get(self: Box<Self>, txn: &Txn, selector: Value) -> TCResult<State> {
        (self.read_fn)(self.tensor, txn, selector)
    }
}

struct SliceHandler<'a, T: TensorInstance> {
    tensor: &'a T,
}

#[async_trait]
impl<'a, T: TensorInstance> Handler for SliceHandler<'a, T> {
    fn subject(&self) -> TCType {
        self.tensor.class().into()
    }

    fn scope(&self) -> Option<Scope> {
        Some(SCOPE_READ.into())
    }

    async fn handle_get(self: Box<Self>, txn: &Txn, selector: Value) -> TCResult<State> {
        let bounds = if selector.is_none() {
            Bounds::all(self.tensor.shape())
        } else {
            selector.try_cast_into(|s| error::bad_request("Expected Tensor bounds but found", s))?
        };

        if let Some(coord) = bounds.as_coord() {
            let value = self.tensor.read_value(&txn, coord).await?;
            Ok(State::Scalar(Scalar::Value(Value::Number(value))))
        } else {
            let slice = self.tensor.slice(bounds)?;
            Ok(State::Collection(slice.into_view().into()))
        }
    }

    async fn handle_post(
        self: Box<Self>,
        _request: &Request,
        _txn: &Txn,
        mut params: Map<Scalar>,
    ) -> TCResult<State> {
        let bounds = params
            .remove(&label("bounds").into())
            .ok_or(error::bad_request("Missing parameter", "bounds"))?;
        let bounds = Bounds::from_scalar(self.tensor.shape(), bounds)?;

        if params.is_empty() {
            self.tensor
                .slice(bounds)
                .map(IntoView::into_view)
                .map(Collection::from)
                .map(State::Collection)
        } else {
            Err(error::bad_request(
                "Unrecognized parameters",
                Scalar::from_iter(params.into_inner()),
            ))
        }
    }
}

struct WriteHandler<'a, T: TensorInstance> {
    tensor: &'a T,
}

#[async_trait]
impl<'a, T: TensorInstance + TensorDualIO<Tensor>> Handler for WriteHandler<'a, T> {
    fn subject(&self) -> TCType {
        self.tensor.class().into()
    }

    fn scope(&self) -> Option<Scope> {
        Some(SCOPE_WRITE.into())
    }

    async fn handle_put(
        self: Box<Self>,
        _request: &Request,
        txn: &Txn,
        selector: Value,
        value: State,
    ) -> TCResult<()> {
        let bounds = if selector.is_none() {
            Bounds::all(self.tensor.shape())
        } else {
            selector.try_cast_into(|s| error::bad_request("Expected Tensor bounds but found", s))?
        };

        match value {
            State::Scalar(Scalar::Value(Value::Number(value))) => {
                self.tensor
                    .write_value(txn.id().clone(), bounds, value)
                    .await
            }
            State::Collection(Collection::Tensor(tensor)) => {
                self.tensor.write(txn, bounds, tensor).await
            }
            other => Err(error::bad_request(
                "Not a valid Tensor value or slice",
                other,
            )),
        }
    }
}

pub fn route<'a, T: TensorInstance + TensorDualIO<Tensor>>(
    tensor: &'a T,
    method: MethodType,
    path: &'_ [PathSegment],
) -> Option<Box<dyn Handler + 'a>> {
    if path.is_empty() {
        let handler: Box<dyn Handler> = match method {
            MethodType::Get => Box::new(SliceHandler { tensor }),
            MethodType::Put => Box::new(WriteHandler { tensor }),
            _ => return None,
        };

        Some(handler)
    } else if path.len() == 1 {
        let handler: Box<dyn Handler> = match path[0].as_str() {
            "all" => Box::new(AllHandler { tensor }),
            "any" => Box::new(AnyHandler { tensor }),
            "as_type" => Box::new(GetHandler {
                tensor,
                read_fn: |tensor, _txn, selector| {
                    let dtype: NumberType =
                        selector.try_cast_into(|v| error::bad_request("Invalid NumberType", v))?;

                    tensor
                        .as_type(dtype)
                        .map(IntoView::into_view)
                        .map(Collection::from)
                        .map(State::Collection)
                },
            }),
            "broadcast" => Box::new(GetHandler {
                tensor,
                read_fn: |tensor, _txn, selector| {
                    let shape =
                        selector.try_cast_into(|v| error::bad_request("Invalid shape", v))?;

                    tensor
                        .broadcast(shape)
                        .map(IntoView::into_view)
                        .map(Collection::from)
                        .map(State::Collection)
                },
            }),
            "expand_dims" => Box::new(GetHandler {
                tensor,
                read_fn: |tensor, _txn, selector| {
                    let axis = selector.try_cast_into(|v| error::bad_request("Invalid axis", v))?;

                    tensor
                        .expand_dims(axis)
                        .map(IntoView::into_view)
                        .map(Collection::from)
                        .map(State::Collection)
                },
            }),
            "not" => Box::new(GetHandler {
                tensor,
                read_fn: |tensor, _txn, selector| {
                    if selector.is_none() {
                        tensor
                            .not()
                            .map(IntoView::into_view)
                            .map(Collection::from)
                            .map(State::Collection)
                    } else {
                        Err(error::bad_request(
                            "Tensor::not takes no parameters, found",
                            selector,
                        ))
                    }
                },
            }),
            "slice" => Box::new(SliceHandler { tensor }),
            "transpose" => Box::new(GetHandler {
                tensor,
                read_fn: |tensor, _txn, selector| {
                    let permutation = if selector.is_none() {
                        None
                    } else {
                        let permutation = selector.try_cast_into(|v| {
                            error::bad_request("Permutation should be a tuple of axes, not", v)
                        })?;
                        Some(permutation)
                    };

                    tensor
                        .transpose(permutation)
                        .map(IntoView::into_view)
                        .map(Collection::from)
                        .map(State::Collection)
                },
            }),
            _ => return None,
        };

        Some(handler)
    } else {
        None
    }
}
