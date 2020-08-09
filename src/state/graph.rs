use std::convert::TryInto;
use std::sync::Arc;

use async_trait::async_trait;
use futures::future::{self, join_all};
use futures::stream::{FuturesOrdered, StreamExt, TryStreamExt};
use futures::try_join;

use crate::error;
use crate::transaction::lock::{Mutable, TxnLock};
use crate::transaction::{Transact, Txn, TxnId};
use crate::value::class::NumberType;
use crate::value::{Number, TCResult, TCTryStream, UInt, Value};

use super::table;
use super::tensor::{self, einsum, SparseTensor, TensorBoolean, TensorIO};

const ERR_CORRUPT: &str = "Graph corrupted! Please file a bug report.";

pub struct Graph {
    nodes: table::TableBase,
    edges: table::TableBase,
    max_id: TxnLock<Mutable<u64>>,
}

impl Graph {
    pub async fn create(txn: Arc<Txn>, node_schema: Vec<table::Column>) -> TCResult<Graph> {
        // TODO: replace incrementing numeric IDs with UUIDs

        let key: Vec<table::Column> = vec![("id", NumberType::uint64()).try_into()?];
        let nodes = table::Table::create(txn.clone(), (key, node_schema).into()).await?;

        let max_id = 0u64;
        let shape: tensor::Shape = vec![max_id, max_id].into();
        let edges =
            tensor::SparseTable::create_table(txn.clone(), shape.len(), NumberType::Bool).await?;
        let max_id = TxnLock::new(txn.id().clone(), 0u64.into());

        Ok(Graph {
            nodes,
            edges,
            max_id,
        })
    }

    async fn get_matrix(&self, txn_id: &TxnId) -> TCResult<SparseTensor> {
        let max_id = self.max_id.read(txn_id).await?;
        let shape: tensor::Shape = vec![*max_id, *max_id].into();
        SparseTensor::try_from_table(self.edges.clone(), shape)
    }

    pub async fn add_node(&self, txn_id: TxnId, node: Vec<Value>) -> TCResult<()> {
        let mut max_id = self.max_id.write(txn_id.clone()).await?;
        self.nodes
            .insert(txn_id, vec![u64_value(*max_id)], node)
            .await?;
        *max_id += 1;
        Ok(())
    }

    pub async fn add_edge(&self, txn_id: TxnId, node_from: u64, node_to: u64) -> TCResult<()> {
        let edges = self.get_matrix(&txn_id).await?;
        edges
            .write_value_at(txn_id, vec![node_from, node_to], true.into())
            .await
    }

    pub async fn bft(&self, txn: Arc<Txn>, start_node: u64) -> TCResult<TCTryStream<Vec<Value>>> {
        let edges = self.get_matrix(txn.id());
        let max_id = self.max_id.read(txn.id());
        let (edges, max_id) = try_join!(edges, max_id)?;

        let visited = SparseTensor::create(txn.clone(), vec![*max_id].into(), NumberType::Bool);
        let adjacent = SparseTensor::create(txn.clone(), vec![*max_id].into(), NumberType::Bool);
        let (mut visited, mut adjacent) = try_join!(visited, adjacent)?;
        adjacent
            .write_value_at(txn.id().clone(), vec![start_node], true.into())
            .await?;

        // TODO: stream the search itself instead of buffering these futures
        let mut found = FuturesOrdered::new();

        while adjacent.any(txn.clone()).await? {
            visited = visited.or(&adjacent)?;
            adjacent = einsum("ji,j->i", vec![edges.clone(), adjacent])?.and(&visited.not()?)?;

            let txn_id = txn.id().clone();
            let nodes = self.nodes.clone();

            let adjacent_nodes = adjacent
                .clone()
                .filled(txn.clone())
                .await?
                .and_then(move |(id, _)| {
                    nodes
                        .clone()
                        .get_owned(txn_id.clone(), vec![u64_value(id[0])])
                })
                .map(|r| r.and_then(|node| node.ok_or_else(|| error::internal(ERR_CORRUPT))));
            found.push(future::ready(adjacent_nodes));
        }

        let found: TCTryStream<Vec<Value>> = Box::pin(found.flatten());
        Ok(found)
    }
}

#[async_trait]
impl Transact for Graph {
    async fn commit(&self, txn_id: &TxnId) {
        join_all(vec![
            self.nodes.commit(txn_id),
            self.edges.commit(txn_id),
            self.max_id.commit(txn_id),
        ])
        .await;
    }

    async fn rollback(&self, txn_id: &TxnId) {
        join_all(vec![
            self.nodes.rollback(txn_id),
            self.edges.rollback(txn_id),
            self.max_id.rollback(txn_id),
        ])
        .await;
    }
}

fn u64_value(value: u64) -> Value {
    Value::Number(Number::UInt(UInt::U64(value)))
}
