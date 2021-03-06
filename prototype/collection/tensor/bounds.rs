use std::fmt;
use std::iter::{self, FromIterator};
use std::ops::{self, Deref, DerefMut};

use itertools::{Itertools, MultiProduct};

use crate::error;
use crate::scalar::{Bound, Scalar, Slice, Value};
use crate::{Match, TCResult, TryCastFrom, TryCastInto};

use super::Coord;

pub type Coords = MultiProduct<AxisIter>;

#[derive(Clone)]
pub enum AxisIter {
    One(std::iter::Once<u64>),
    Each(Vec<u64>, usize),
    Step(iter::StepBy<ops::Range<u64>>),
}

impl Iterator for AxisIter {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        use AxisIter::*;
        match self {
            One(iter) => iter.next(),
            Each(v, at) => {
                if at == &v.len() {
                    None
                } else {
                    Some(v[*at])
                }
            }
            Step(iter) => iter.next(),
        }
    }
}

#[derive(Clone)]
pub enum AxisBounds {
    At(u64),
    In(ops::Range<u64>),
    Of(Vec<u64>),
}

impl AxisBounds {
    pub fn all(dim: u64) -> AxisBounds {
        AxisBounds::In(0..dim)
    }

    pub fn dim(&self) -> u64 {
        match self {
            Self::At(_) => 0,
            Self::In(range) => range.end - range.start,
            Self::Of(indices) => indices.len() as u64,
        }
    }

    pub fn is_index(&self) -> bool {
        if let Self::At(_) = self {
            true
        } else {
            false
        }
    }
}

impl PartialEq for AxisBounds {
    fn eq(&self, other: &AxisBounds) -> bool {
        use AxisBounds::*;
        match (self, other) {
            (At(l), At(r)) if l == r => true,
            (In(lr), In(rr)) if lr == rr => true,
            (Of(l), Of(r)) if l == r => true,
            _ => false,
        }
    }
}

impl From<u64> for AxisBounds {
    fn from(at: u64) -> AxisBounds {
        AxisBounds::At(at)
    }
}

impl From<Vec<u64>> for AxisBounds {
    fn from(of: Vec<u64>) -> AxisBounds {
        AxisBounds::Of(of)
    }
}

impl From<ops::Range<u64>> for AxisBounds {
    fn from(range: ops::Range<u64>) -> AxisBounds {
        AxisBounds::In(range)
    }
}

impl TryCastFrom<Value> for AxisBounds {
    fn can_cast_from(value: &Value) -> bool {
        value.matches::<u64>() || value.matches::<(u64, u64)>() || value.matches::<Vec<u64>>()
    }

    fn opt_cast_from(value: Value) -> Option<AxisBounds> {
        if value.matches::<u64>() {
            value.opt_cast_into().map(AxisBounds::At)
        } else if value.matches::<(u64, u64)>() {
            let range: (u64, u64) = value.opt_cast_into().unwrap();
            Some(AxisBounds::In(range.0..range.1))
        } else if value.matches::<Vec<u64>>() {
            value.opt_cast_into().map(AxisBounds::Of)
        } else {
            None
        }
    }
}

impl fmt::Display for AxisBounds {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use AxisBounds::*;
        match self {
            At(at) => write!(f, "{}", at),
            In(range) => write!(f, "[{}, {})", range.start, range.end),
            Of(indices) => write!(
                f,
                "{{{}}}",
                indices
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<String>>()
                    .join(", ")
            ),
        }
    }
}

#[derive(Clone)]
pub struct Bounds {
    pub axes: Vec<AxisBounds>,
}

impl Bounds {
    fn cast_bound(dim: u64, bound: Value) -> TCResult<u64> {
        let bound = i64::try_cast_from(bound, |v| error::bad_request("Invalid bound", v))?;
        if bound.abs() as u64 > dim {
            return Err(error::bad_request(
                format!("Index out of bounds for dimension {}", dim),
                bound,
            ));
        }

        if bound < 0 {
            Ok(dim - bound.abs() as u64)
        } else {
            Ok(bound as u64)
        }
    }

    pub fn from_scalar(shape: &Shape, scalar: Scalar) -> TCResult<Bounds> {
        match scalar {
            Scalar::Tuple(bounds) => {
                let mut axes = Vec::with_capacity(shape.len());

                for (axis, bound) in bounds.into_inner().into_iter().enumerate() {
                    let bound = match bound {
                        bound if bound.is_none() => AxisBounds::In(0..shape[axis]),
                        Scalar::Slice(Slice::Range(range)) => {
                            let start = match range.start {
                                Bound::Unbounded => 0,
                                Bound::In(start) => Self::cast_bound(shape[axis], start)?,
                                Bound::Ex(start) => Self::cast_bound(shape[1], start)? + 1,
                            };

                            let end = match range.end {
                                Bound::Unbounded => shape[axis],
                                Bound::In(end) => Self::cast_bound(shape[axis], end)?,
                                Bound::Ex(end) => Self::cast_bound(shape[1], end)?,
                            };

                            AxisBounds::In(start..end)
                        }
                        Scalar::Value(Value::Tuple(indices)) => {
                            let indices = shape[..]
                                .iter()
                                .zip(indices.into_inner().into_iter())
                                .map(|(dim, i)| Self::cast_bound(*dim, i.into()))
                                .collect::<TCResult<Vec<u64>>>()?;
                            AxisBounds::Of(indices)
                        }
                        Scalar::Value(i) => {
                            let i = Self::cast_bound(shape[axis], i)?;
                            AxisBounds::At(i)
                        }
                        other => {
                            return Err(error::bad_request(
                                format!("Invalid bound for axis {}", axis),
                                other,
                            ));
                        }
                    };

                    axes.push(bound);
                }

                Ok(Bounds { axes })
            }
            Scalar::Value(Value::Tuple(bounds)) => {
                let mut axes = Vec::with_capacity(shape.len());
                for (axis, bound) in bounds.into_inner().into_iter().enumerate() {
                    let bound = match bound {
                        Value::Tuple(indices) => {
                            let indices = shape[..]
                                .iter()
                                .zip(indices.into_inner().into_iter())
                                .map(|(dim, i)| Self::cast_bound(*dim, i.into()))
                                .collect::<TCResult<Vec<u64>>>()?;
                            AxisBounds::Of(indices)
                        }
                        value => {
                            let i = Self::cast_bound(shape[axis], value)?;
                            AxisBounds::At(i)
                        }
                    };

                    axes.push(bound);
                }

                Ok(Bounds { axes })
            }
            other => Err(error::bad_request("Invalid Tensor bounds", other)),
        }
    }

    pub fn all(shape: &Shape) -> Bounds {
        shape
            .0
            .iter()
            .map(|dim| AxisBounds::In(0..*dim))
            .collect::<Vec<AxisBounds>>()
            .into()
    }

    pub fn affected(&self) -> Coords {
        use AxisBounds::*;
        let mut axes = Vec::with_capacity(self.len());
        for axis in 0..self.len() {
            axes.push(match &self[axis] {
                At(i) => AxisIter::One(iter::once(*i)),
                In(range) => AxisIter::Step(range.clone().step_by(1)),
                Of(indices) => AxisIter::Each(indices.to_vec(), 0),
            });
        }

        axes.iter().cloned().multi_cartesian_product()
    }

    pub fn contains_coord(&self, coord: &[u64]) -> bool {
        if coord.len() != self.len() {
            return false;
        }

        use AxisBounds::*;
        for (bound, c) in self.axes.iter().zip(coord) {
            match bound {
                At(i) if i != c => return false,
                In(range) if !range.contains(c) => return false,
                Of(indices) if !indices.contains(c) => return false,
                _ => {}
            }
        }

        true
    }

    pub fn as_coord(&self) -> Option<Coord> {
        let mut coord = Coord::new();
        for bound in &self.axes {
            if let AxisBounds::At(i) = bound {
                coord.push(*i);
            } else {
                return None;
            }
        }

        Some(coord)
    }

    pub fn normalize(&mut self, shape: &Shape) {
        assert!(self.len() <= shape.len());

        for axis in self.axes.len()..shape.len() {
            self.axes.push(AxisBounds::all(shape[axis]))
        }
    }

    pub fn to_shape(&self) -> Shape {
        let mut shape = Vec::with_capacity(self.len());
        for bound in &self.axes {
            let dim = bound.dim();
            if dim > 0 {
                shape.push(dim);
            }
        }

        shape.into()
    }

    pub fn size(&self) -> u64 {
        self.to_shape().size()
    }
}

impl Deref for Bounds {
    type Target = Vec<AxisBounds>;

    fn deref(&'_ self) -> &'_ Self::Target {
        &self.axes
    }
}

impl DerefMut for Bounds {
    fn deref_mut(&'_ mut self) -> &'_ mut Self::Target {
        &mut self.axes
    }
}

impl PartialEq for Bounds {
    fn eq(&self, other: &Self) -> bool {
        self.axes == other.axes
    }
}

impl From<Vec<AxisBounds>> for Bounds {
    fn from(axes: Vec<AxisBounds>) -> Bounds {
        Bounds { axes }
    }
}

impl From<&[u64]> for Bounds {
    fn from(coord: &[u64]) -> Bounds {
        let axes = coord.iter().map(|i| AxisBounds::At(*i)).collect();
        Bounds { axes }
    }
}

impl From<Vec<u64>> for Bounds {
    fn from(coord: Vec<u64>) -> Bounds {
        let axes = coord.iter().map(|i| AxisBounds::At(*i)).collect();
        Bounds { axes }
    }
}

impl From<(Vec<u64>, Vec<u64>)> for Bounds {
    fn from(bounds: (Vec<u64>, Vec<u64>)) -> Bounds {
        bounds
            .0
            .iter()
            .zip(bounds.1.iter())
            .map(|(s, e)| AxisBounds::In(*s..*e))
            .collect::<Vec<AxisBounds>>()
            .into()
    }
}

impl From<(AxisBounds, Vec<u64>)> for Bounds {
    fn from(tuple: (AxisBounds, Vec<u64>)) -> Bounds {
        let mut axes = Vec::with_capacity(tuple.1.len() + 1);
        axes.push(tuple.0);
        for axis in tuple.1.into_iter() {
            axes.push(axis.into());
        }
        Bounds { axes }
    }
}

impl TryCastFrom<Value> for Bounds {
    fn can_cast_from(value: &Value) -> bool {
        value.matches::<Vec<AxisBounds>>()
    }

    fn opt_cast_from(value: Value) -> Option<Bounds> {
        let bounds: Option<Vec<AxisBounds>> = value.opt_cast_into();
        bounds.map(Bounds::from)
    }
}

impl fmt::Display for Bounds {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "[{}]",
            self.axes
                .iter()
                .map(|axis| format!("{}", axis))
                .collect::<Vec<String>>()
                .join(", ")
        )
    }
}

#[derive(Clone, Default)]
pub struct Shape(Vec<u64>);

impl Shape {
    pub fn contains_bounds(&self, bounds: &Bounds) -> bool {
        if bounds.len() > self.len() {
            return false;
        }

        for axis in 0..bounds.len() {
            let size = &self[axis];
            match &bounds[axis] {
                AxisBounds::At(i) => {
                    if i > size {
                        return false;
                    }
                }
                AxisBounds::In(range) => {
                    if range.start > *size || range.end > *size {
                        return false;
                    }
                }
                AxisBounds::Of(indices) => {
                    for i in indices {
                        if i > size {
                            return false;
                        }
                    }
                }
            }
        }

        true
    }

    pub fn contains_coord(&self, coord: &[u64]) -> bool {
        if coord.len() != self.len() {
            return false;
        }

        for axis in 0..coord.len() {
            if coord[axis] >= self[axis] {
                return false;
            }
        }

        true
    }

    pub fn size(&self) -> u64 {
        self.0.iter().product()
    }

    pub fn slice_bounds(&self, mut bounds: Bounds) -> Bounds {
        assert!(bounds.len() <= self.len());

        for axis in bounds.len()..self.len() {
            bounds.push(AxisBounds::In(0..self[axis]));
        }

        bounds
    }

    pub fn validate_bounds(&self, bounds: &Bounds) -> TCResult<()> {
        if self.contains_bounds(bounds) {
            Ok(())
        } else {
            Err(error::unsupported(format!(
                "Tensor of shape {} does not contain bounds {}",
                self, bounds
            )))
        }
    }

    pub fn validate_coord(&self, coord: &[u64]) -> TCResult<()> {
        for (axis, index) in coord.iter().enumerate() {
            if index >= &self[axis] {
                return Err(error::unsupported(format!(
                    "Tensor of shape {} does not contain {}",
                    self,
                    Value::from_iter(coord.to_vec())
                )));
            }
        }

        Ok(())
    }
}

impl Deref for Shape {
    type Target = Vec<u64>;

    fn deref(&'_ self) -> &'_ Vec<u64> {
        &self.0
    }
}

impl DerefMut for Shape {
    fn deref_mut(&'_ mut self) -> &'_ mut Vec<u64> {
        &mut self.0
    }
}

impl PartialEq for Shape {
    fn eq(&self, other: &Shape) -> bool {
        self.0 == other.0
    }
}

impl Eq for Shape {}

impl From<Vec<u64>> for Shape {
    fn from(shape: Vec<u64>) -> Shape {
        Shape(shape)
    }
}

impl TryCastFrom<Value> for Shape {
    fn can_cast_from(value: &Value) -> bool {
        value.matches::<Vec<u64>>()
    }

    fn opt_cast_from(value: Value) -> Option<Shape> {
        let shape: Option<Vec<u64>> = value.opt_cast_into();
        shape.map(Shape::from)
    }
}

impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "[{}]",
            self.0
                .iter()
                .map(|dim| format!("{}", dim))
                .collect::<Vec<String>>()
                .join(", ")
        )
    }
}

impl fmt::Debug for Shape {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
