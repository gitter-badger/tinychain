use std::cmp::Ordering;
use std::convert::TryFrom;
use std::fmt;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures::future::{self, join, try_join, try_join_all, Future};
use futures::stream::{self, FuturesOrdered, Stream, StreamExt, TryStreamExt};
use log::debug;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::block::File;
use crate::block::{Block, BlockData, BlockId, BlockMut};
use crate::class::Instance;
use crate::collection::schema::{Column, RowSchema};
use crate::collection::Collection;
use crate::error;
use crate::scalar::*;
use crate::transaction::lock::{Mutable, TxnLock};
use crate::transaction::{Transact, Txn, TxnId};
use crate::{TCBoxTryFuture, TCResult, TCTryStream};

use super::collator::Collator;
use super::{validate_key, validate_range, BTreeInstance, BTreeRange, BTreeType, Key};

type Selection<'a> = FuturesOrdered<
    Pin<Box<dyn Future<Output = TCResult<TCTryStream<'a, Key>>> + Send + Unpin + 'a>>,
>;

const DEFAULT_BLOCK_SIZE: usize = 4_000;
const BLOCK_ID_SIZE: usize = 128; // UUIDs are 128-bit

type NodeId = BlockId;

#[derive(Clone, Deserialize, Serialize)]
struct NodeKey {
    value: Vec<Value>,
    deleted: bool,
}

impl Deref for NodeKey {
    type Target = [Value];

    fn deref(&self) -> &[Value] {
        &self.value
    }
}

impl From<&[Value]> for NodeKey {
    fn from(values: &[Value]) -> NodeKey {
        values.to_vec().into()
    }
}

impl From<Vec<Value>> for NodeKey {
    fn from(value: Vec<Value>) -> NodeKey {
        NodeKey {
            value,
            deleted: false,
        }
    }
}

impl fmt::Display for NodeKey {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "BTree node key: {}{}",
            Value::Tuple(self.value.to_vec().into()),
            if self.deleted { " (DELETED)" } else { "" }
        )
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Node {
    leaf: bool,
    keys: Vec<NodeKey>,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    rebalance: bool, // TODO: implement rebalancing to clear deleted values
}

impl Node {
    fn new(leaf: bool, parent: Option<NodeId>) -> Node {
        Node {
            leaf,
            keys: vec![],
            parent,
            children: vec![],
            rebalance: false,
        }
    }
}

impl TryFrom<Bytes> for Node {
    type Error = error::TCError;

    fn try_from(serialized: Bytes) -> TCResult<Node> {
        bincode::deserialize(&serialized).map_err(|e| e.into())
    }
}

impl From<Node> for Bytes {
    fn from(node: Node) -> Bytes {
        bincode::serialize(&node).unwrap().into()
    }
}

impl BlockData for Node {
    fn size(&self) -> usize {
        self.keys.len() + self.children.len()
    }
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.leaf {
            writeln!(f, "leaf node:")?;
        } else {
            writeln!(f, "non-leaf node:")?;
        }

        write!(
            f,
            "\tkeys: {}",
            self.keys
                .iter()
                .map(|k| k.to_string())
                .collect::<Vec<String>>()
                .join(", ")
        )?;

        write!(f, "\t {} children", self.children.len())
    }
}

#[derive(Clone)]
pub struct BTreeFile {
    file: Arc<File<Node>>,
    schema: RowSchema,
    order: usize,
    collator: Collator,
    root: TxnLock<Mutable<NodeId>>,
}

impl BTreeFile {
    pub async fn create(txn: &Txn, schema: RowSchema) -> TCResult<Self> {
        let file = txn.context().await?;

        if !file.is_empty(txn.id()).await? {
            return Err(error::internal(
                "Tried to create a new BTree without a new File",
            ));
        }

        let mut key_size = 0;
        for col in &schema {
            if let Some(size) = col.dtype().size() {
                key_size += size;
                if col.max_len().is_some() {
                    return Err(error::bad_request(
                        "Found maximum length specified for a scalar type",
                        col.dtype(),
                    ));
                }
            } else if let Some(size) = col.max_len() {
                key_size += size + 8; // add 8 bytes for bincode to encode the length
            } else {
                return Err(error::bad_request(
                    "Type requires a maximum length",
                    col.dtype(),
                ));
            }
        }
        // the "leaf" and "deleted" booleans each add one byte to a key as-stored
        key_size += 2;

        let order = if DEFAULT_BLOCK_SIZE > (key_size * 2) + (BLOCK_ID_SIZE * 3) {
            // let m := order
            // maximum block size = (m * key_size) + ((m + 1) * block_id_size)
            // therefore block_size = (m * (key_size + block_id_size)) + block_id_size
            // therefore block_size - block_id_size = m * (key_size + block_id_size)
            // therefore m = floor((block_size - block_id_size) / (key_size + block_id_size))
            (DEFAULT_BLOCK_SIZE - BLOCK_ID_SIZE) / (key_size + BLOCK_ID_SIZE)
        } else {
            2
        };

        let root: BlockId = Uuid::new_v4().into();
        file.clone()
            .create_block(txn.id().clone(), root.clone(), Node::new(true, None))
            .await?;

        let collator = Collator::new(schema.iter().map(|c| c.dtype()).collect())?;

        Ok(BTreeFile {
            file,
            schema,
            order,
            collator,
            root: TxnLock::new("BTree root", root.into()),
        })
    }

    pub fn collator(&'_ self) -> &'_ Collator {
        &self.collator
    }

    fn _slice<'a>(
        &'a self,
        txn_id: &'a TxnId,
        node: Block<'a, Node>,
        range: BTreeRange,
    ) -> TCResult<TCTryStream<'a, Key>> {
        let (l, r) = bisect(&range, &node.keys[..], &self.collator);

        debug!("_slice {} from {} to {}", node.deref(), l, r);

        if node.leaf {
            let stream: TCTryStream<'a, Key> = if l == r && l < node.keys.len() {
                if !node.keys[l].deleted
                    && self
                        .collator
                        .contains(range.start(), range.end(), &node.keys[l])
                {
                    let key = TCResult::Ok(node.keys[l].value.to_vec());
                    Box::pin(stream::once(future::ready(key)))
                } else {
                    Box::pin(stream::empty())
                }
            } else {
                let keys = node.keys[l..r]
                    .iter()
                    .filter(|k| !k.deleted)
                    .map(|k| k.value.to_vec())
                    .map(TCResult::Ok)
                    .collect::<Vec<TCResult<Key>>>();

                Box::pin(stream::iter(keys))
            };

            Ok(stream)
        } else {
            let mut selected: Selection<'a> = FuturesOrdered::new();
            for i in l..r {
                let child_id = node.children[i].clone();
                let range_clone = range.clone();

                let selection = Box::pin(async move {
                    let node = self.file.get_block(&txn_id, child_id).await?;

                    self._slice(txn_id, node, range_clone)
                });
                selected.push(Box::pin(selection));

                if !node.keys[i].deleted {
                    let key_at_i = TCResult::Ok(node.keys[i].value.to_vec());
                    let key_at_i: TCTryStream<'a, Key> =
                        Box::pin(stream::once(future::ready(key_at_i)));

                    selected.push(Box::pin(future::ready(Ok(key_at_i))));
                }
            }

            let last_child_id = node.children[r].clone();

            let selection = Box::pin(async move {
                let node: Block<'a, Node> = self.file.get_block(&txn_id, last_child_id).await?;

                self._slice(txn_id, node, range)
            });
            selected.push(Box::pin(selection));

            Ok(Box::pin(selected.try_flatten()))
        }
    }

    fn _slice_reverse<'a>(
        &'a self,
        txn_id: &'a TxnId,
        node: Block<'a, Node>,
        range: BTreeRange,
    ) -> TCResult<TCTryStream<'a, Key>> {
        let (l, r) = bisect(&range, &node.keys, &self.collator);

        debug!("_slice_reverse {} from {} to {}", node.deref(), l, r);

        if node.leaf {
            let keys = node.keys[l..r]
                .iter()
                .filter(|k| !k.deleted)
                .rev()
                .map(|k| k.value.to_vec())
                .map(TCResult::Ok)
                .collect::<Vec<TCResult<Key>>>();

            Ok(Box::pin(stream::iter(keys)))
        } else {
            let mut selected: Selection<'a> = FuturesOrdered::new();

            let last_child = node.children[r].clone();
            let range_clone = range.clone();
            let selection = Box::pin(async move {
                let node: Block<'a, Node> = self.file.get_block(txn_id, last_child).await?;

                self._slice_reverse(txn_id, node, range_clone)
            });
            selected.push(Box::pin(selection));

            for i in (l..r).rev() {
                let child_id = node.children[i].clone();
                let range_clone = range.clone();

                let selection = Box::pin(async move {
                    let node: Block<'a, Node> = self.file.get_block(txn_id, child_id).await?;

                    self._slice_reverse(txn_id, node, range_clone)
                });

                if !node.keys[i].deleted {
                    let key_at_i = TCResult::Ok(node.keys[i].value.to_vec());
                    let key_at_i: TCTryStream<'a, Key> =
                        Box::pin(stream::once(future::ready(key_at_i)));
                    selected.push(Box::pin(future::ready(Ok(key_at_i))));
                }

                selected.push(Box::pin(selection));
            }

            Ok(Box::pin(selected.try_flatten()))
        }
    }

    pub async fn update(&self, txn_id: &TxnId, range: BTreeRange, value: &[Value]) -> TCResult<()> {
        let range = validate_range(range, self.schema())?;
        let root_id = self.root.read(txn_id).await?;
        self._update(txn_id, &root_id, &range, value).await
    }

    fn _update<'a>(
        &'a self,
        txn_id: &'a TxnId,
        node_id: &'a NodeId,
        range: &'a BTreeRange,
        value: &'a [Value],
    ) -> TCBoxTryFuture<'a, ()> {
        Box::pin(async move {
            let node = self.file.get_block(txn_id, node_id.clone()).await?;
            let (l, r) = bisect(range, &node.keys, &self.collator);

            if node.leaf {
                if l == r {
                    return Ok(());
                }

                let mut node = node.upgrade().await?;
                for i in l..r {
                    node.keys[i] = value.into();
                }

                Ok(())
            } else {
                let children = node.children.to_vec();

                if r > l {
                    let mut node = node.upgrade().await?;
                    let mut updates = Vec::with_capacity(r - l);
                    for (i, child_id) in children.iter().enumerate().take(r).skip(l) {
                        node.keys[i] = value.into();
                        updates.push(self._update(txn_id, child_id, range, value));
                    }

                    let last_update = self._update(txn_id, &children[r], range, value);
                    try_join(try_join_all(updates), last_update).await?;
                    Ok(())
                } else {
                    self._update(txn_id, &children[r], range, value).await
                }
            }
        })
    }

    fn _insert<'a>(
        &'a self,
        txn_id: &'a TxnId,
        node: Block<'a, Node>,
        key: Key,
    ) -> TCBoxTryFuture<'a, ()> {
        Box::pin(async move {
            let i = self.collator.bisect_left(&node.keys, &key);
            if i < node.keys.len() && self.collator.compare(&node.keys[i], &key) == Ordering::Equal
            {
                if node.keys[i].deleted {
                    let mut node = node.upgrade().await?;
                    node.keys[i].deleted = false;
                }

                return Ok(());
            }

            debug!("insert at index {} into {}", i, node.deref());

            if node.leaf {
                let mut node = node.upgrade().await?;
                node.keys.insert(i, key.into());
                Ok(())
            } else {
                let mut child = self
                    .file
                    .get_block(txn_id, node.children[i].clone())
                    .await?;

                if child.keys.len() == (2 * self.order) - 1 {
                    let node = self
                        .split_child(txn_id, node.children[i].clone(), node.upgrade().await?, i)
                        .await?;

                    match self.collator.compare(&key, &node.keys[i]) {
                        Ordering::Less => {}
                        Ordering::Equal => {
                            if node.keys[i].deleted {
                                let mut node = node.upgrade().await?;
                                node.keys[i].deleted = false;
                            }

                            return Ok(());
                        }
                        Ordering::Greater => {
                            child = self
                                .file
                                .get_block(txn_id, node.children[i + 1].clone())
                                .await?;
                        }
                    }
                }

                self._insert(txn_id, child, key).await
            }
        })
    }

    async fn split_child<'a>(
        &'a self,
        txn_id: &'a TxnId,
        node_id: NodeId,
        mut node: BlockMut<'a, Node>,
        i: usize,
    ) -> TCResult<Block<'a, Node>> {
        let child_id = node.children[i].clone(); // needed due to mutable borrow below
        let mut child = self
            .file
            .get_block(txn_id, child_id)
            .await?
            .upgrade()
            .await?;

        debug!(
            "child to split has {} keys and {} children",
            child.keys.len(),
            child.children.len()
        );

        let new_node_id = self.file.unique_id(&txn_id).await?;

        node.children.insert(i + 1, new_node_id.clone());
        node.keys.insert(i, child.keys.remove(self.order - 1));

        let mut new_node = Node::new(child.leaf, Some(node_id));
        new_node.keys = child.keys.drain((self.order - 1)..).collect();

        if child.leaf {
            debug!("child is a leaf node");
        } else {
            new_node.children = child.children.drain(self.order..).collect();
        }

        self.file
            .clone()
            .create_block(*txn_id, new_node_id, new_node)
            .await?;

        node.downgrade(&txn_id).await
    }

    fn _delete<'a>(
        &'a self,
        txn_id: &'a TxnId,
        node_id: NodeId,
        range: &'a BTreeRange,
    ) -> TCBoxTryFuture<'a, ()> {
        Box::pin(async move {
            let node = self.file.get_block(txn_id, node_id).await?;
            let (l, r) = bisect(range, &node.keys, &self.collator);

            debug!("delete from {} [{}..{}]", node.deref(), l, r);

            if node.leaf {
                if l == r {
                    return Ok(());
                }

                let mut node = node.upgrade().await?;
                for i in l..r {
                    node.keys[i].deleted = true;
                }
                node.rebalance = true;

                Ok(())
            } else if r > l {
                let mut node = node.upgrade().await?;
                let mut deletes = Vec::with_capacity(r - l);

                for i in l..r {
                    node.keys[i].deleted = true;
                    deletes.push(self._delete(txn_id, node.children[i].clone(), range));
                }
                node.rebalance = true;

                let last_delete = self._delete(txn_id, node.children[r].clone(), range);
                try_join(try_join_all(deletes), last_delete).await?;

                Ok(())
            } else {
                self._delete(txn_id, node.children[r].clone(), range).await
            }
        })
    }

    async fn assert_valid(&self, txn_id: &TxnId) -> TCResult<()> {
        use num::integer::div_ceil;
        use std::collections::VecDeque;

        let root_id = self.root.read(txn_id).await?;
        let root = self.file.get_block(txn_id, root_id.deref().clone()).await?;
        let order = self.order;

        assert!(self.collator.is_sorted(&root.keys));
        assert!(root.children.len() <= 2 * order);
        if !root.leaf {
            assert!(root.children.len() >= 2);
        }

        let mut unvisited: VecDeque<NodeId> = root.children.iter().cloned().collect();
        while let Some(node_id) = unvisited.pop_front() {
            let node = self.file.get_block(txn_id, node_id).await?;

            assert!(!node.keys.is_empty());
            assert!(self.collator.is_sorted(&node.keys));
            assert!(node.children.len() <= 2 * order);

            if node.leaf {
                assert!(node.children.is_empty());
            } else {
                assert_eq!(node.children.len(), node.keys.len() + 1);
                assert!(node.children.len() >= div_ceil(order, 2));

                for i in 0..node.keys.len() {
                    let child_at_i = self
                        .file
                        .get_block(txn_id, node.children[i].clone())
                        .await?;

                    let child_after_i = self
                        .file
                        .get_block(txn_id, node.children[i + 1].clone())
                        .await?;

                    assert!(!child_at_i.keys.is_empty());
                    assert!(!child_after_i.keys.is_empty());
                    assert_eq!(
                        self.collator
                            .compare(child_at_i.keys.last().unwrap(), &node.keys[i]),
                        Ordering::Less
                    );
                    assert_eq!(
                        self.collator.compare(&child_after_i.keys[0], &node.keys[i]),
                        Ordering::Greater
                    );
                }
            }

            unvisited.extend(node.children.iter().cloned());
            debug!("node is valid");
        }

        Ok(())
    }
}

impl Instance for BTreeFile {
    type Class = BTreeType;

    fn class(&self) -> Self::Class {
        BTreeType::Tree
    }
}

#[async_trait]
impl BTreeInstance for BTreeFile {
    async fn delete(&self, txn_id: &TxnId, range: BTreeRange) -> TCResult<()> {
        let range = validate_range(range, self.schema())?;
        let root_id = self.root.read(txn_id).await?;
        self._delete(txn_id, (*root_id).clone(), &range).await
    }

    async fn insert(&self, txn_id: &TxnId, key: Key) -> TCResult<()> {
        let root_id = self.root.read(txn_id).await?;
        let root = self.file.get_block(txn_id, root_id.deref().clone()).await?;

        debug!(
            "insert into BTree node with {} keys and {} children (order is {})",
            root.keys.len(),
            root.children.len(),
            self.order
        );

        if root.keys.len() == (2 * self.order) - 1 {
            let mut root_id = root_id.upgrade().await?;
            let old_root_id = (*root_id).clone();

            (*root_id) = self.file.unique_id(&txn_id).await?;
            let mut new_root = Node::new(false, None);
            new_root.children.push(old_root_id.clone());

            self.file
                .clone()
                .create_block(*txn_id, (*root_id).clone(), new_root)
                .await?;

            let new_root = self
                .file
                .get_block(txn_id, root_id.deref().clone())
                .await?
                .upgrade()
                .await?;
            let new_root = self.split_child(txn_id, old_root_id, new_root, 0).await?;
            self._insert(txn_id, new_root, key).await
        } else {
            self._insert(txn_id, root, key).await
        }
    }

    async fn insert_from<S: Stream<Item = Key> + Send>(
        &self,
        txn_id: &TxnId,
        source: S,
    ) -> TCResult<()> {
        source
            .map(|k| validate_key(k, self.schema()))
            .map_ok(|key| self.insert(txn_id, key))
            .try_buffer_unordered(2 * self.order)
            .fold(Ok(()), |_, r| future::ready(r))
            .await
    }

    async fn try_insert_from<S: Stream<Item = TCResult<Key>> + Send>(
        &self,
        txn_id: &TxnId,
        source: S,
    ) -> TCResult<()> {
        source
            .and_then(|k| future::ready(validate_key(k, self.schema())))
            .map_ok(|key| self.insert(txn_id, key))
            .try_buffer_unordered(2 * self.order)
            .fold(Ok(()), |_, r| future::ready(r))
            .await
    }

    async fn is_empty(&self, txn: &Txn) -> TCResult<bool> {
        let root_id = self.root.read(txn.id()).await?;
        let root = self
            .file
            .get_block(txn.id(), root_id.deref().clone())
            .await?;
        Ok(root.keys.is_empty())
    }

    async fn len(&self, txn_id: &TxnId, range: BTreeRange) -> TCResult<u64> {
        let slice = self.stream(txn_id, range, false).await?;
        Ok(slice.fold(0u64, |len, _| future::ready(len + 1)).await)
    }

    fn schema(&'_ self) -> &'_ [Column] {
        &self.schema
    }

    async fn stream<'a>(
        &'a self,
        txn_id: &'a TxnId,
        range: BTreeRange,
        reverse: bool,
    ) -> TCResult<TCTryStream<'a, Key>> {
        let range = validate_range(range, self.schema())?;

        let root_id = self.root.read(txn_id).await?;
        let root: Block<'a, Node> = self.file.get_block(txn_id, root_id.deref().clone()).await?;

        if reverse {
            self._slice_reverse(txn_id, root, range)
        } else {
            self._slice(txn_id, root, range)
        }
    }
}

#[async_trait]
impl Transact for BTreeFile {
    async fn commit(&self, txn_id: &TxnId) {
        join(self.file.commit(txn_id), self.root.commit(txn_id)).await;
    }

    async fn rollback(&self, txn_id: &TxnId) {
        join(self.file.rollback(txn_id), self.root.rollback(txn_id)).await;
    }

    async fn finalize(&self, txn_id: &TxnId) {
        join(self.file.finalize(txn_id), self.root.finalize(txn_id)).await;
    }
}

impl From<BTreeFile> for Collection {
    fn from(btree: BTreeFile) -> Collection {
        Collection::BTree(btree.into())
    }
}

fn bisect<V: Deref<Target = [Value]>>(
    range: &BTreeRange,
    keys: &[V],
    collator: &Collator,
) -> (usize, usize) {
    (
        collator.bisect_left_range(keys, range.start()),
        collator.bisect_right_range(keys, range.end()),
    )
}
