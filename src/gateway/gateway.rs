use std::sync::Arc;

use crate::auth::{Auth, Token};
use crate::block::dir::Dir;
use crate::class::{State, TCResult};
use crate::error;
use crate::kernel;
use crate::transaction::{Txn, TxnId};
use crate::value::link::Link;
use crate::value::Value;

use super::{Hosted, NetworkTime};

pub struct Gateway {
    hosted: Hosted,
    workspace: Arc<Dir>,
}

impl Gateway {
    pub fn time() -> NetworkTime {
        NetworkTime::now()
    }

    pub fn new(hosted: Hosted, workspace: Arc<Dir>) -> Arc<Gateway> {
        Arc::new(Gateway { hosted, workspace })
    }

    pub async fn authenticate(&self, _token: &str) -> TCResult<Token> {
        Err(error::not_implemented())
    }

    pub async fn transaction(self: &Arc<Self>) -> TCResult<Arc<Txn>> {
        Txn::new(self.clone(), self.workspace.clone()).await
    }

    pub async fn get(
        &self,
        subject: &Link,
        selector: Value,
        _auth: &Auth,
        _txn_id: Option<TxnId>,
    ) -> TCResult<State> {
        if subject.host().is_none() {
            let path = subject.path();
            if path[0] == "sbin" {
                kernel::get(&path.slice_from(1), selector)
            } else {
                Err(error::not_implemented())
            }
        } else if let Some((_rel_path, _cluster)) = self.hosted.get(subject.path()) {
            Err(error::not_implemented())
        } else {
            Err(error::not_implemented())
        }
    }

    pub async fn put(
        &self,
        _subject: &Link,
        _selector: Value,
        _state: State,
        _txn_id: TxnId,
        _auth: &Auth,
    ) -> TCResult<State> {
        Err(error::not_implemented())
    }
}
