use std::convert::{TryFrom, TryInto};
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;

use crate::class::{Class, Instance, TCResult, TCStream, TCType};
use crate::error;
use crate::transaction::{Transact, Txn};
use crate::value::link::{Link, TCPath};
use crate::value::{label, Value};

use super::btree::{BTreeFile, BTreeType};
use super::table::TableBaseType;
use super::{Collection, CollectionBase, CollectionView};

#[async_trait]
pub trait CollectionClass: Class + Into<CollectionType> + Send + Sync {
    type Instance: CollectionInstance;

    async fn get(
        txn: Arc<Txn>,
        path: &TCPath,
        schema: Value,
    ) -> TCResult<<Self as CollectionClass>::Instance>;
}

#[async_trait]
pub trait CollectionInstance: Instance + Into<Collection> + Transact + Send + Sync {
    type Error: Into<error::TCError>;
    type Item: Into<Value> + TryFrom<Value, Error = Self::Error>;
    type Slice: CollectionInstance;

    async fn get(&self, txn: Arc<Txn>, selector: Value) -> TCResult<Self::Slice>;

    async fn is_empty(&self, txn: Arc<Txn>) -> TCResult<bool>;

    async fn put(&self, txn: Arc<Txn>, selector: Value, value: Self::Item) -> TCResult<()>;

    async fn to_stream(&self, txn: Arc<Txn>) -> TCResult<TCStream<Self::Item>>;
}

#[derive(Clone, Eq, PartialEq)]
pub enum CollectionType {
    Base(CollectionBaseType),
    View(CollectionViewType),
}

impl Class for CollectionType {
    type Instance = Collection;

    fn from_path(path: &TCPath) -> TCResult<Self> {
        CollectionBaseType::from_path(path).map(CollectionType::Base)
    }

    fn prefix() -> TCPath {
        TCType::prefix().join(label("collection").into())
    }
}

#[async_trait]
impl CollectionClass for CollectionType {
    type Instance = Collection;

    async fn get(
        txn: Arc<Txn>,
        path: &TCPath,
        schema: Value,
    ) -> TCResult<<Self as CollectionClass>::Instance> {
        CollectionBaseType::get(txn, path, schema)
            .await
            .map(Collection::Base)
    }
}

impl From<CollectionBaseType> for CollectionType {
    fn from(cbt: CollectionBaseType) -> CollectionType {
        CollectionType::Base(cbt)
    }
}

impl From<CollectionViewType> for CollectionType {
    fn from(cvt: CollectionViewType) -> CollectionType {
        CollectionType::View(cvt)
    }
}

impl From<CollectionType> for Link {
    fn from(ct: CollectionType) -> Link {
        use CollectionType::*;
        match ct {
            Base(base) => base.into(),
            View(view) => view.into(),
        }
    }
}

impl fmt::Display for CollectionType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CollectionType::Base(base) => write!(f, "{}", base),
            CollectionType::View(view) => write!(f, "{}", view),
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum CollectionBaseType {
    BTree,
    Graph,
    Table(TableBaseType),
    Tensor,
}

impl Class for CollectionBaseType {
    type Instance = CollectionBase;

    fn from_path(path: &TCPath) -> TCResult<Self> {
        let suffix = path.from_path(&Self::prefix())?;

        if suffix.is_empty() {
            Err(error::unsupported("You must specify a type of Collection"))
        } else {
            use CollectionBaseType::*;
            match suffix[0].as_str() {
                "btree" if suffix.len() == 1 => Ok(BTree),
                "graph" if suffix.len() == 1 => Ok(Graph),
                "table" => TableBaseType::from_path(path).map(Table),
                "tensor" if suffix.len() == 1 => Ok(Tensor),
                other => Err(error::not_found(other)),
            }
        }
    }

    fn prefix() -> TCPath {
        CollectionType::prefix()
    }
}

#[async_trait]
impl CollectionClass for CollectionBaseType {
    type Instance = CollectionBase;

    async fn get(txn: Arc<Txn>, path: &TCPath, schema: Value) -> TCResult<CollectionBase> {
        if path.is_empty() {
            return Err(error::unsupported("You must specify a type of Collection"));
        }

        match path[0].as_str() {
            "btree" if path.len() == 1 => {
                println!("Creating new BTree with schema {}", schema);
                BTreeFile::create(txn, schema.try_into()?)
                    .await
                    .map(CollectionBase::BTree)
            }
            "graph" | "table" | "tensor" => Err(error::not_implemented(&format!(
                "CollectionBaseType::get {}",
                path
            ))),
            other => Err(error::not_found(other)),
        }
    }
}

impl From<CollectionBaseType> for Link {
    fn from(ct: CollectionBaseType) -> Link {
        let prefix = CollectionBaseType::prefix();

        use CollectionBaseType::*;
        match ct {
            BTree => BTreeType::Tree.into(),
            Graph => prefix.join(label("graph").into()).into(), // TODO
            Table(tbt) => tbt.into(),
            Tensor => prefix.join(label("tensor").into()).into(), // TODO
        }
    }
}

impl fmt::Display for CollectionBaseType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use CollectionBaseType::*;
        match self {
            BTree => write!(f, "class BTree"),
            Graph => write!(f, "class Graph"),
            Table(tbt) => write!(f, "{}", tbt),
            Tensor => write!(f, "class Tensor"),
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum CollectionViewType {
    BTree(BTreeType),
    Graph,
    Table,
    Tensor,
}

impl Class for CollectionViewType {
    type Instance = CollectionView;

    fn from_path(_path: &TCPath) -> TCResult<Self> {
        Err(error::internal(crate::class::ERR_PROTECTED))
    }

    fn prefix() -> TCPath {
        CollectionType::prefix()
    }
}

impl From<BTreeType> for CollectionViewType {
    fn from(btt: BTreeType) -> CollectionViewType {
        CollectionViewType::BTree(btt)
    }
}

impl From<CollectionViewType> for Link {
    fn from(cvt: CollectionViewType) -> Link {
        let prefix = CollectionViewType::prefix();

        use CollectionViewType::*;
        match cvt {
            BTree(btt) => btt.into(),
            Graph => prefix.join(label("graph").into()).into(), // TODO
            Table => prefix.join(label("table").into()).into(), // TODO
            Tensor => prefix.join(label("tensor").into()).into(), // TODO
        }
    }
}

impl fmt::Display for CollectionViewType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use CollectionViewType::*;
        match self {
            BTree(btree_type) => write!(f, "{}", btree_type),
            Graph => write!(f, "class Graph"),       // TODO
            Table => write!(f, "class TableView"),   // TODO
            Tensor => write!(f, "class TensorView"), // TODO
        }
    }
}
