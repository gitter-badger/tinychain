use std::collections::HashMap;
use std::fmt;
use std::iter::FromIterator;
use std::ops::{Deref, DerefMut};

use async_trait::async_trait;
use destream::de::{Decoder, FromStream};

use super::Id;

#[derive(Clone)]
pub struct Map<T: Clone> {
    inner: HashMap<Id, T>,
}

impl<T: Clone> Map<T> {
    pub fn into_inner(self) -> HashMap<Id, T> {
        self.inner
    }
}

impl<T: Clone> Default for Map<T> {
    fn default() -> Map<T> {
        HashMap::new().into()
    }
}

impl<T: Clone + PartialEq> PartialEq for Map<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<T: Clone + PartialEq + Eq> Eq for Map<T> {}

impl<T: Clone> AsRef<HashMap<Id, T>> for Map<T> {
    fn as_ref(&self) -> &HashMap<Id, T> {
        &self.inner
    }
}

impl<T: Clone> Deref for Map<T> {
    type Target = HashMap<Id, T>;

    fn deref(&'_ self) -> &'_ Self::Target {
        &self.inner
    }
}

impl<T: Clone> DerefMut for Map<T> {
    fn deref_mut(&'_ mut self) -> &'_ mut <Self as Deref>::Target {
        &mut self.inner
    }
}

impl<T: Clone> IntoIterator for Map<T> {
    type Item = (Id, T);
    type IntoIter = <HashMap<Id, T> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl<T: Clone> FromIterator<(Id, T)> for Map<T> {
    fn from_iter<I: IntoIterator<Item = (Id, T)>>(iter: I) -> Self {
        let inner = HashMap::from_iter(iter);
        Map { inner }
    }
}

impl<T: Clone> From<HashMap<Id, T>> for Map<T> {
    fn from(inner: HashMap<Id, T>) -> Self {
        Map { inner }
    }
}

#[async_trait]
impl<T: Clone + FromStream> FromStream for Map<T> {
    async fn from_stream<D: Decoder>(d: &mut D) -> Result<Self, D::Error> {
        let inner = HashMap::<Id, T>::from_stream(d).await?;
        Ok(Self { inner })
    }
}

impl<T: Clone + fmt::Display> fmt::Display for Map<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{{{}}}",
            self.inner
                .iter()
                .map(|(k, v)| format!("{}: {}", k, v))
                .collect::<Vec<String>>()
                .join(", ")
        )
    }
}
