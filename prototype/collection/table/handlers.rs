use std::iter::FromIterator;
use std::ops::Deref;

use async_trait::async_trait;
use futures::stream::TryStreamExt;
use futures::TryFutureExt;

use crate::auth::{Scope, SCOPE_READ, SCOPE_WRITE};
use crate::class::{Instance, State, TCType};
use crate::collection::CollectionInstance;
use crate::error;
use crate::general::Map;
use crate::handler::*;
use crate::request::Request;
use crate::scalar::{Id, MethodType, PathSegment, Scalar, Value};
use crate::transaction::Txn;
use crate::{Match, TCResult, TCTryStream, TryCastFrom, TryCastInto};

use super::{Bounds, Table, TableInstance};

pub struct DeleteHandler<'a, T: TableInstance> {
    table: &'a T,
}

#[async_trait]
impl<'a, T: TableInstance> Handler for DeleteHandler<'a, T>
where
    <T as Instance>::Class: Into<TCType>,
{
    fn subject(&self) -> TCType {
        self.table.class().into()
    }

    fn scope(&self) -> Option<Scope> {
        Some(SCOPE_WRITE.into())
    }

    async fn handle_delete(self: Box<Self>, txn: &Txn, key: Value) -> TCResult<()> {
        if key.is_none() {
            self.table.delete(txn.id()).await
        } else {
            Err(error::bad_request(
                "Table::delete expected no arguments but found",
                key,
            ))
        }
    }
}

pub struct GroupByHandler<'a, T: TableInstance> {
    table: &'a T,
}

#[async_trait]
impl<'a, T: TableInstance> Handler for GroupByHandler<'a, T>
where
    <T as Instance>::Class: Into<TCType>,
{
    fn subject(&self) -> TCType {
        self.table.class().into()
    }

    fn scope(&self) -> Option<Scope> {
        Some(SCOPE_READ.into())
    }

    async fn handle_get(self: Box<Self>, _txn: &Txn, selector: Value) -> TCResult<State> {
        let columns: Vec<Id> = try_into_columns(selector)?;
        self.table
            .clone()
            .group_by(columns)
            .map(TableInstance::into_table)
            .map(State::from)
    }
}

pub struct InsertHandler<'a, T: TableInstance> {
    table: &'a T,
}

#[async_trait]
impl<'a, T: TableInstance> Handler for InsertHandler<'a, T>
where
    <T as Instance>::Class: Into<TCType>,
{
    fn subject(&self) -> TCType {
        self.table.class().into()
    }

    fn scope(&self) -> Option<Scope> {
        Some(SCOPE_WRITE.into())
    }

    async fn handle_put(
        self: Box<Self>,
        _request: &Request,
        txn: &Txn,
        key: Value,
        value: State,
    ) -> TCResult<()> {
        let (key, values) = try_into_row(key, value)?;
        self.table.insert(txn.id(), key, values).await
    }
}

pub struct LimitHandler<'a, T: TableInstance> {
    table: &'a T,
}

#[async_trait]
impl<'a, T: TableInstance> Handler for LimitHandler<'a, T>
where
    <T as Instance>::Class: Into<TCType>,
{
    fn subject(&self) -> TCType {
        self.table.class().into()
    }

    fn scope(&self) -> Option<Scope> {
        Some(SCOPE_READ.into())
    }

    async fn handle_get(self: Box<Self>, _txn: &Txn, selector: Value) -> TCResult<State> {
        let limit = selector.try_cast_into(|v| error::bad_request("Invalid limit", v))?;
        Ok(State::from(self.table.clone().limit(limit).into_table()))
    }
}

pub struct OrderByHandler<'a, T: TableInstance> {
    table: &'a T,
}

#[async_trait]
impl<'a, T: TableInstance> Handler for OrderByHandler<'a, T>
where
    <T as Instance>::Class: Into<TCType>,
{
    fn subject(&self) -> TCType {
        self.table.class().into()
    }

    fn scope(&self) -> Option<Scope> {
        Some(SCOPE_READ.into())
    }

    async fn handle_get(self: Box<Self>, _txn: &Txn, selector: Value) -> TCResult<State> {
        let columns: Vec<Id> = try_into_columns(selector)?;
        self.table
            .clone()
            .order_by(columns, false)
            .map(TableInstance::into_table)
            .map(State::from)
    }
}

pub struct ReverseHandler<'a, T: TableInstance> {
    table: &'a T,
}

#[async_trait]
impl<'a, T: TableInstance> Handler for ReverseHandler<'a, T>
where
    <T as Instance>::Class: Into<TCType>,
{
    fn subject(&self) -> TCType {
        self.table.class().into()
    }

    fn scope(&self) -> Option<Scope> {
        Some(SCOPE_READ.into())
    }

    async fn handle_get(self: Box<Self>, _txn: &Txn, selector: Value) -> TCResult<State> {
        if selector.is_none() {
            self.table
                .clone()
                .reversed()
                .map(TableInstance::into_table)
                .map(State::from)
        } else {
            Err(error::bad_request(
                "Table::reverse takes no arguments but found",
                selector,
            ))
        }
    }
}

pub struct SelectHandler<'a, T: TableInstance> {
    table: &'a T,
}

#[async_trait]
impl<'a, T: TableInstance> Handler for SelectHandler<'a, T>
where
    <T as Instance>::Class: Into<TCType>,
{
    fn subject(&self) -> TCType {
        self.table.class().into()
    }

    fn scope(&self) -> Option<Scope> {
        Some(SCOPE_READ.into())
    }

    async fn handle_get(self: Box<Self>, _txn: &Txn, selector: Value) -> TCResult<State> {
        let columns = try_into_columns(selector)?;
        self.table
            .clone()
            .select(columns)
            .map(TableInstance::into_table)
            .map(State::from)
    }
}

pub struct UpdateHandler<'a, T: TableInstance> {
    table: &'a T,
}

#[async_trait]
impl<'a, T: TableInstance> Handler for UpdateHandler<'a, T>
where
    <T as Instance>::Class: Into<TCType>,
{
    fn subject(&self) -> TCType {
        self.table.class().into()
    }

    fn scope(&self) -> Option<Scope> {
        Some(SCOPE_WRITE.into())
    }

    async fn handle_post(
        self: Box<Self>,
        _request: &Request,
        txn: &Txn,
        params: Map<Scalar>,
    ) -> TCResult<State> {
        let update = params.try_cast_into(|v| error::bad_request("Invalid update", v))?;

        self.table.update(txn, update).map_ok(State::from).await
    }
}

pub struct UpsertHandler<'a, T: TableInstance> {
    table: &'a T,
}

#[async_trait]
impl<'a, T: TableInstance> Handler for UpsertHandler<'a, T>
where
    <T as Instance>::Class: Into<TCType>,
{
    fn subject(&self) -> TCType {
        self.table.class().into()
    }

    fn scope(&self) -> Option<Scope> {
        Some(SCOPE_WRITE.into())
    }

    async fn handle_put(
        self: Box<Self>,
        _request: &Request,
        txn: &Txn,
        key: Value,
        value: State,
    ) -> TCResult<()> {
        let (key, values) = try_into_row(key, value)?;
        self.table.upsert(txn.id(), key, values).await
    }
}

pub struct WhereHandler<'a, T: TableInstance> {
    table: &'a T,
}

#[async_trait]
impl<'a, T: TableInstance> Handler for WhereHandler<'a, T>
where
    <T as Instance>::Class: Into<TCType>,
{
    fn subject(&self) -> TCType {
        self.table.class().into()
    }

    fn scope(&self) -> Option<Scope> {
        Some(SCOPE_READ.into())
    }

    async fn handle_get(self: Box<Self>, txn: &Txn, selector: Value) -> TCResult<State> {
        if selector.is_none() {
            let table: Table = self.table.clone().into_table();
            Ok(State::from(table))
        } else {
            let key: Vec<Value> =
                selector.try_cast_into(|v| error::bad_request("Invalid key for Table", v))?;
            if key.len() == self.table.key().len() {
                let bounds = Bounds::from_key(key, self.table.key());
                let slice = self.table.clone().slice(bounds)?;
                let mut stream = slice.stream(txn.id()).await?;
                let next = stream.try_next().await?;

                next.map(Value::from_iter)
                    .map(Scalar::Value)
                    .map(State::Scalar)
                    .ok_or_else(|| error::not_found("(table row)"))
            } else {
                Err(error::bad_request(
                    format!(
                        "Table key has {} columns, so the specified key is not valid",
                        self.table.key().len()
                    ),
                    Value::from_iter(key),
                ))
            }
        }
    }

    async fn handle_post(
        self: Box<Self>,
        _request: &Request,
        _txn: &Txn,
        params: Map<Scalar>,
    ) -> TCResult<State> {
        let bounds = Bounds::try_cast_from(params, |v| {
            error::bad_request("Cannot cast into Table Bounds from", v)
        })?;

        self.table
            .clone()
            .slice(bounds)
            .map(TableInstance::into_table)
            .map(State::from)
    }
}
#[derive(Clone)]
pub struct TableImpl<T: TableInstance> {
    inner: T,
}

impl<T: TableInstance> TableImpl<T> {
    pub fn into_inner(self) -> T {
        self.inner
    }

    pub fn into_table(self) -> Table {
        self.inner.into_table()
    }
}

#[async_trait]
impl<T: TableInstance> CollectionInstance for TableImpl<T> {
    type Item = Vec<Value>;

    async fn is_empty(&self, txn: &Txn) -> TCResult<bool> {
        let mut rows = self.inner.stream(txn.id()).await?;
        if let Some(_row) = rows.try_next().await? {
            Ok(false)
        } else {
            Ok(true)
        }
    }

    async fn to_stream<'a>(&'a self, txn: &'a Txn) -> TCResult<TCTryStream<'a, Scalar>> {
        let stream = self.inner.stream(txn.id()).await?;
        Ok(Box::pin(stream.map_ok(Scalar::from)))
    }
}

#[async_trait]
impl<T: TableInstance> Route for TableImpl<T>
where
    <T as Instance>::Class: Into<TCType>,
{
    fn route(
        &'_ self,
        method: MethodType,
        path: &'_ [PathSegment],
    ) -> Option<Box<dyn Handler + '_>> {
        let table = &self.inner;

        if path.is_empty() {
            let handler: Box<dyn Handler> = match method {
                MethodType::Get => Box::new(WhereHandler { table }),
                MethodType::Put => Box::new(UpsertHandler { table }),
                MethodType::Post => Box::new(WhereHandler { table }),
                MethodType::Delete => Box::new(DeleteHandler { table }),
            };

            Some(handler)
        } else if path.len() == 1 {
            let handler: Box<dyn Handler> = match path[0].as_str() {
                "group_by" => Box::new(GroupByHandler { table }),
                "insert" => Box::new(InsertHandler { table }),
                "limit" => Box::new(LimitHandler { table }),
                "order_by" => Box::new(OrderByHandler { table }),
                "reverse" => Box::new(ReverseHandler { table }),
                "select" => Box::new(SelectHandler { table }),
                "update" => Box::new(UpdateHandler { table }),
                "where" => Box::new(WhereHandler { table }),
                _ => return None,
            };

            Some(handler)
        } else {
            None
        }
    }
}

impl<T: TableInstance> Deref for TableImpl<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T: TableInstance> From<T> for TableImpl<T> {
    fn from(inner: T) -> TableImpl<T> {
        Self { inner }
    }
}

fn try_into_row(selector: Value, values: State) -> TCResult<(Vec<Value>, Vec<Value>)> {
    let key = match selector {
        Value::Tuple(key) => key.into_inner(),
        other => vec![other],
    };

    let values = Value::try_cast_from(values, |v| error::bad_request("Invalid row value", v))?;
    let values = match values {
        Value::Tuple(values) => values.into_inner(),
        other => vec![other],
    };

    Ok((key, values))
}

fn try_into_columns(selector: Value) -> TCResult<Vec<Id>> {
    if selector.matches::<Vec<Id>>() {
        Ok(selector.opt_cast_into().unwrap())
    } else {
        let name = selector.try_cast_into(|v| error::bad_request("Invalid column name", v))?;

        Ok(vec![name])
    }
}
