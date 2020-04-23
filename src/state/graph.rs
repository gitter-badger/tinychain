use std::sync::Arc;

use async_trait::async_trait;

use crate::error;
use crate::state::Persistent;
use crate::transaction::{Transaction, TransactionId};
use crate::value::{TCResult, TCValue};

#[derive(Debug)]
pub struct Graph {}

#[async_trait]
impl Persistent for Graph {
    type Key = TCValue;
    type Value = TCValue;

    async fn commit(self: &Arc<Self>, _txn_id: TransactionId) {
        // TODO
    }

    async fn get(
        self: &Arc<Self>,
        _txn: Arc<Transaction>,
        _node_id: &TCValue,
    ) -> TCResult<Self::Value> {
        Err(error::not_implemented())
    }

    async fn put(
        self: &Arc<Self>,
        _txn: Arc<Transaction>,
        _node_id: TCValue,
        _node: TCValue,
    ) -> TCResult<Arc<Self>> {
        Err(error::not_implemented())
    }
}
