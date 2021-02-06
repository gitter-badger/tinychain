use std::collections::HashSet;
use std::convert::TryInto;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use async_trait::async_trait;
use futures::future::join_all;
use futures_locks::RwLock;
use log::debug;
use uuid::Uuid;

use error::*;
use generic::PathSegment;

use crate::lock::{Mutable, TxnLock, TxnLockReadGuard};
use crate::{Transact, TxnId};

use super::cache::Cache;
use super::hostfs;
use super::{Block, BlockData, BlockId, BlockOwned};

const ERR_CORRUPT: &str = "Data corruption error detected! Please file a bug report.";
const TXN_CACHE: &str = ".pending";

struct Inner<T: BlockData> {
    dir: RwLock<hostfs::Dir>,
    pending: RwLock<hostfs::Dir>,
    listing: TxnLock<Mutable<HashSet<BlockId>>>,
    cache: RwLock<Cache<T>>,
    mutated: TxnLock<Mutable<HashSet<BlockId>>>,
}

#[derive(Clone)]
pub struct File<T: BlockData> {
    inner: Arc<Inner<T>>,
}

impl<T: BlockData> File<T> {
    pub async fn create(name: &str, dir: RwLock<hostfs::Dir>) -> TCResult<File<T>> {
        let mut lock = dir.write().await;
        if !lock.is_empty() {
            return Err(TCError::bad_request(
                "Tried to create a new File but there is already data in the cache!",
                "(filesystem cache)",
            ));
        }

        let inner = Inner {
            dir,
            pending: lock.create_dir(TXN_CACHE.parse()?).await?,
            listing: TxnLock::new(format!("File listing for {}", name), HashSet::new().into()),
            cache: RwLock::new(Cache::new()),
            mutated: TxnLock::new("File mutated contents".to_string(), HashSet::new().into()),
        };

        Ok(File {
            inner: Arc::new(inner),
        })
    }

    pub async fn unique_id(&self, txn_id: &TxnId) -> TCResult<BlockId> {
        let existing_ids = self.block_ids(txn_id).await?;
        loop {
            let id: PathSegment = Uuid::new_v4().to_string().parse()?;
            if !existing_ids.contains(&id) {
                return Ok(id);
            }
        }
    }

    async fn block_ids(&'_ self, txn_id: &'_ TxnId) -> TCResult<HashSet<BlockId>> {
        self.inner
            .listing
            .read(txn_id)
            .await
            .map(|block_ids| block_ids.clone())
    }

    pub async fn mutate(&self, txn_id: TxnId, block_id: BlockId) -> TCResult<()> {
        self.inner.mutated.write(txn_id).await?.insert(block_id);
        Ok(())
    }

    pub async fn create_block(
        self,
        txn_id: TxnId,
        block_id: BlockId,
        data: T,
    ) -> TCResult<BlockOwned<T>> {
        if &block_id == TXN_CACHE {
            return Err(TCError::bad_request("This name is reserved", block_id));
        }

        let mut listing = self.inner.listing.write(txn_id).await?;
        if listing.contains(&block_id) {
            return Err(TCError::bad_request(
                "There is already a block called",
                block_id,
            ));
        }
        listing.insert(block_id.clone());

        let txn_lock = self
            .inner
            .cache
            .write()
            .await
            .insert(block_id.clone(), data);

        let lock = txn_lock.read(&txn_id).await?;
        Ok(BlockOwned::new(self, block_id, lock))
    }

    pub async fn get_block<'a>(
        &'a self,
        txn_id: &'a TxnId,
        block_id: BlockId,
    ) -> TCResult<Block<'a, T>> {
        let lock = self.lock_block(txn_id, &block_id).await?;
        let block = Block::new(self, block_id, lock);
        Ok(block)
    }

    pub async fn get_block_owned(
        self,
        txn_id: TxnId,
        block_id: BlockId,
    ) -> TCResult<BlockOwned<T>> {
        let lock = self.lock_block(&txn_id, &block_id).await?;
        Ok(BlockOwned::new(self, block_id, lock))
    }

    async fn lock_block(
        &self,
        txn_id: &TxnId,
        block_id: &BlockId,
    ) -> TCResult<TxnLockReadGuard<T>> {
        if let Some(block) = self.inner.cache.read().await.get(block_id) {
            block.read(txn_id).await
        } else if self.inner.listing.read(txn_id).await?.contains(block_id) {
            let txn_dir = self.inner.pending.read().await.get_dir(&txn_id.to_id())?;
            let block = if let Some(txn_dir) = txn_dir {
                if let Some(block) = txn_dir.read().await.get_block(block_id).await? {
                    block
                } else {
                    self.inner
                        .dir
                        .read()
                        .await
                        .get_block(&block_id)
                        .await?
                        .ok_or_else(|| TCError::internal(ERR_CORRUPT))?
                }
            } else {
                self.inner
                    .dir
                    .read()
                    .await
                    .get_block(&block_id)
                    .await?
                    .ok_or_else(|| TCError::internal(ERR_CORRUPT))?
            };

            let block: T = block.try_into()?;
            let txn_lock = self
                .inner
                .cache
                .write()
                .await
                .insert(block_id.clone(), block);

            txn_lock.read(txn_id).await
        } else {
            Err(TCError::not_found(block_id))
        }
    }

    pub async fn is_empty(&self, txn_id: &TxnId) -> TCResult<bool> {
        let listing = self.inner.listing.read(txn_id).await?;
        Ok(listing.is_empty())
    }
}

#[async_trait]
impl<T: BlockData> Transact for File<T> {
    async fn commit(&self, txn_id: &TxnId) {
        let this = &self.inner;

        let new_listing = this.listing.read(txn_id).await.unwrap();
        let old_listing = this.listing.canonical().value();

        let mut dir = this.dir.write().await;
        for block_id in old_listing.difference(&new_listing) {
            dir.delete_block(block_id).await.unwrap();
        }

        this.listing.commit(txn_id).await;

        let mutated: Vec<BlockId> = this.mutated.write(*txn_id).await.unwrap().drain().collect();
        this.mutated.commit(txn_id).await;

        let cache = this.cache.read().await;
        debug!("File::commit! cache has {} blocks", cache.len());
        if mutated.is_empty() {
            cache.commit(txn_id).await;
            return;
        }

        let mut pending = this.pending.write().await;
        let txn_dir = pending.create_or_get_dir(&txn_id.to_id()).await.unwrap();

        let copy_ops = mutated
            .into_iter()
            .filter_map(|block_id| cache.get(&block_id).map(|lock| (block_id, lock)))
            .map(|(block_id, lock)| {
                let dir_lock = txn_dir.write();
                async move {
                    let data = lock.read(txn_id).await.unwrap().deref().clone().into();
                    debug!(
                        "copying block {} from cache to Txn dir ({} bytes)",
                        &block_id,
                        data.len()
                    );

                    dir_lock.await.create_block(block_id, data).await.unwrap();
                }
            });

        join_all(copy_ops).await;
        cache.commit(txn_id).await;
        debug!("emptied cache");
        dir.copy_all(txn_dir.write().await.deref_mut())
            .await
            .unwrap();
        debug!("copied all blocks to main Dir");
    }

    async fn finalize(&self, txn_id: &TxnId) {
        let mut pending = self.inner.pending.write().await;
        pending.delete_dir(&txn_id.to_id()).await.unwrap();

        self.inner.listing.finalize(txn_id).await;
    }
}
