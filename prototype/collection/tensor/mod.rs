use async_trait::async_trait;

use crate::error;
use crate::scalar::value::number::*;
use crate::transaction::{Txn, TxnId};
use crate::{TCBoxTryFuture, TCResult};

mod einsum;
mod handlers;
mod stream;
mod transform;

pub mod bounds;
pub mod class;
pub mod dense;
pub mod sparse;

pub use bounds::*;
pub use class::{Tensor, TensorInstance, TensorType};
pub use dense::{from_sparse, Array, DenseTensor};
pub use einsum::einsum;
pub use sparse::{from_dense, SparseTensor};

pub type Coord = Vec<u64>;

pub const ERR_NONBIJECTIVE_WRITE: &str = "Cannot write to a derived Tensor which is not a \
bijection of its source. Consider copying first, or writing directly to the source Tensor.";

pub trait IntoView {
    fn into_view(self) -> Tensor;
}

pub trait TensorAccess: Send {
    fn dtype(&self) -> NumberType;

    fn ndim(&self) -> usize;

    fn shape(&'_ self) -> &'_ Shape;

    fn size(&self) -> u64;
}

#[async_trait]
pub trait TensorBoolean<O>: TensorAccess + Sized {
    type Combine: TensorInstance;

    fn and(&self, other: &O) -> TCResult<Self::Combine>;

    fn or(&self, other: &O) -> TCResult<Self::Combine>;

    fn xor(&self, other: &O) -> TCResult<Self::Combine>;
}

#[async_trait]
pub trait TensorUnary: TensorAccess + Sized {
    type Unary: TensorInstance;

    fn abs(&self) -> TCResult<Self::Unary>;

    async fn all(&self, txn: &Txn) -> TCResult<bool>;

    async fn any(&self, txn: &Txn) -> TCResult<bool>;

    fn not(&self) -> TCResult<Self::Unary>;
}

#[async_trait]
pub trait TensorCompare<O>: TensorAccess + Sized {
    type Compare: TensorInstance;
    type Dense: TensorInstance;

    async fn eq(&self, other: &O, txn: &Txn) -> TCResult<Self::Dense>;

    fn gt(&self, other: &O) -> TCResult<Self::Compare>;

    async fn gte(&self, other: &O, txn: &Txn) -> TCResult<Self::Dense>;

    fn lt(&self, other: &O) -> TCResult<Self::Compare>;

    async fn lte(&self, other: &O, txn: &Txn) -> TCResult<Self::Dense>;

    fn ne(&self, other: &O) -> TCResult<Self::Compare>;
}

#[async_trait]
pub trait TensorIO: TensorAccess + Sized {
    async fn read_value(&self, txn: &Txn, coord: Coord) -> TCResult<Number>;

    async fn write_value(
        &self,
        txn_id: TxnId,
        bounds: bounds::Bounds,
        value: Number,
    ) -> TCResult<()>;

    async fn write_value_at(&self, txn_id: TxnId, coord: Coord, value: Number) -> TCResult<()>;
}

#[async_trait]
pub trait TensorDualIO<O>: TensorAccess + Sized {
    async fn mask(&self, txn: &Txn, value: O) -> TCResult<()>;

    async fn write(&self, txn: &Txn, bounds: bounds::Bounds, value: O) -> TCResult<()>;
}

pub trait TensorMath<O>: TensorAccess + Sized {
    type Combine: TensorInstance;

    fn add(&self, other: &O) -> TCResult<Self::Combine>;

    fn multiply(&self, other: &O) -> TCResult<Self::Combine>;
}

pub trait TensorReduce: TensorAccess + Sized {
    type Reduce: TensorInstance;

    fn product(&self, axis: usize) -> TCResult<Self::Reduce>;

    fn product_all(&self, txn: Txn) -> TCBoxTryFuture<Number>;

    fn sum(&self, axis: usize) -> TCResult<Self::Reduce>;

    fn sum_all(&self, txn: Txn) -> TCBoxTryFuture<Number>;
}

pub trait TensorTransform: TensorAccess + Sized {
    type Cast: TensorInstance;
    type Broadcast: TensorInstance;
    type Expand: TensorInstance;
    type Slice: TensorInstance;
    type Transpose: TensorInstance;

    fn as_type(&self, dtype: NumberType) -> TCResult<Self::Cast>;

    fn broadcast(&self, shape: bounds::Shape) -> TCResult<Self::Broadcast>;

    fn expand_dims(&self, axis: usize) -> TCResult<Self::Expand>;

    fn slice(&self, bounds: bounds::Bounds) -> TCResult<Self::Slice>;

    fn transpose(&self, permutation: Option<Vec<usize>>) -> TCResult<Self::Transpose>;
}

fn broadcast<L: Clone + TensorTransform, R: Clone + TensorTransform>(
    left: &L,
    right: &R,
) -> TCResult<(
    <L as TensorTransform>::Broadcast,
    <R as TensorTransform>::Broadcast,
)> {
    let mut left_shape = left.shape().to_vec();
    let mut right_shape = right.shape().to_vec();

    match (left_shape.len(), right_shape.len()) {
        (l, r) if l < r => {
            for _ in 0..(r - l) {
                left_shape.insert(0, 1);
            }
        }
        (l, r) if r < l => {
            for _ in 0..(l - r) {
                right_shape.insert(0, 1);
            }
        }
        _ => {}
    }

    let mut shape = Vec::with_capacity(left_shape.len());
    for (l, r) in left_shape.iter().zip(right_shape.iter()) {
        if l == r || *l == 1 {
            shape.push(*r);
        } else if *r == 1 {
            shape.push(*l)
        } else {
            return Err(error::bad_request(
                "Cannot broadcast dimension",
                format!("{} into {}", l, r),
            ));
        }
    }
    let left = left.broadcast(shape.to_vec().into())?;
    let right = right.broadcast(shape.into())?;
    Ok((left, right))
}
