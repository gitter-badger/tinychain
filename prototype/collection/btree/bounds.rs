use std::cmp::Ordering;
use std::fmt;

use crate::collection::schema::Column;
use crate::error;
use crate::scalar::*;
use crate::{Match, TCResult, TryCastFrom, TryCastInto};

use super::collator::Collator;
use super::Key;

#[derive(Clone, Eq, PartialEq)]
pub struct BTreeRange(Vec<Bound>, Vec<Bound>);

impl BTreeRange {
    pub fn contains(&self, other: &BTreeRange, schema: &[Column], collator: &Collator) -> bool {
        use Bound::*;
        use Ordering::*;

        for (col, (outer, inner)) in schema.iter().zip(self.0.iter().zip(&other.0)) {
            let dtype = col.dtype();

            match (outer, inner) {
                (Unbounded, _) => {}
                (_, Unbounded) => return false,
                (Ex(o), Ex(i)) if collator.compare_value(dtype, &o, &i) == Greater => return false,
                (In(o), In(i)) if collator.compare_value(dtype, &o, &i) == Greater => return false,
                (In(o), Ex(i)) if collator.compare_value(dtype, &o, &i) == Greater => return false,
                (Ex(o), In(i)) if collator.compare_value(dtype, &o, &i) != Less => return false,
                _ => {}
            }
        }

        for (col, (outer, inner)) in schema.iter().zip(self.1.iter().zip(&other.1)) {
            let dtype = col.dtype();

            match (outer, inner) {
                (Unbounded, _) => {}
                (_, Unbounded) => return false,
                (Ex(o), Ex(i)) if collator.compare_value(dtype, &o, &i) == Less => return false,
                (In(o), In(i)) if collator.compare_value(dtype, &o, &i) == Less => return false,
                (In(o), Ex(i)) if collator.compare_value(dtype, &o, &i) == Less => return false,
                (Ex(o), In(i)) if collator.compare_value(dtype, &o, &i) != Greater => return false,
                _ => {}
            }
        }

        true
    }

    pub fn is_key(&self, schema: &[Column]) -> bool {
        self.0.len() == self.1.len()
            && self.0.len() == schema.len()
            && self.0.iter().zip(self.1.iter()).all(|(l, r)| l == r)
    }

    pub fn start(&'_ self) -> &'_ [Bound] {
        &self.0
    }

    pub fn end(&'_ self) -> &'_ [Bound] {
        &self.1
    }
}

pub fn validate_range<T: fmt::Display>(range: T, schema: &[Column]) -> TCResult<BTreeRange>
where
    BTreeRange: TryCastFrom<T>,
{
    use Bound::*;

    let range = BTreeRange::try_cast_from(range, |v| error::bad_request("Invalid BTreeRange", v))?;

    let cast = |(bound, column): (Bound, &Column)| {
        let value = match bound {
            Unbounded => Unbounded,
            In(value) => In(column.dtype().try_cast(value)?),
            Ex(value) => Ex(column.dtype().try_cast(value)?),
        };
        Ok(value)
    };

    let cast_range = |range: Vec<Bound>| {
        range
            .into_iter()
            .zip(schema)
            .map(cast)
            .collect::<TCResult<Vec<Bound>>>()
    };

    let start = cast_range(range.0)?;
    let end = cast_range(range.1)?;
    Ok(BTreeRange(start, end))
}

impl Default for BTreeRange {
    fn default() -> Self {
        Self(vec![], vec![])
    }
}

impl From<Key> for BTreeRange {
    fn from(key: Key) -> Self {
        let start = key.iter().cloned().map(Bound::In).collect();
        let end = key.into_iter().map(Bound::In).collect();
        Self(start, end)
    }
}

impl From<(Vec<Bound>, Vec<Bound>)> for BTreeRange {
    fn from(params: (Vec<Bound>, Vec<Bound>)) -> Self {
        Self(params.0, params.1)
    }
}

impl From<Vec<Range>> for BTreeRange {
    fn from(range: Vec<Range>) -> Self {
        Self::from(range.into_iter().map(Range::into_inner).unzip())
    }
}

impl TryCastFrom<Value> for BTreeRange {
    fn can_cast_from(value: &Value) -> bool {
        if value == &Value::None || Key::can_cast_from(value) {
            true
        } else if let Value::Tuple(tuple) = value {
            tuple.iter().all(|v| v.is_none())
        } else {
            false
        }
    }

    fn opt_cast_from(value: Value) -> Option<BTreeRange> {
        if value == Value::None {
            Some(BTreeRange::default())
        } else if let Value::Tuple(tuple) = value {
            if tuple.iter().all(|v| v.is_none()) {
                Some(BTreeRange::default())
            } else {
                None
            }
        } else {
            Key::opt_cast_from(value).map(BTreeRange::from)
        }
    }
}

impl TryCastFrom<Scalar> for BTreeRange {
    fn can_cast_from(scalar: &Scalar) -> bool {
        match scalar {
            range if range.matches::<Vec<Range>>() => true,
            key if key.matches::<Key>() => true,
            _ => false,
        }
    }

    fn opt_cast_from(scalar: Scalar) -> Option<BTreeRange> {
        match scalar {
            range if range.matches::<Vec<Range>>() => {
                let range: Vec<Range> = range.opt_cast_into().unwrap();
                Some(Self::from(range))
            }
            key if key.matches::<Key>() => {
                let key = Key::opt_cast_from(key).unwrap();
                Some(Self::from(key))
            }
            _ => None,
        }
    }
}

impl fmt::Display for BTreeRange {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.0.is_empty() && self.1.is_empty() {
            return write!(f, "BTreeRange::default");
        }

        let to_str = |bounds: &[Bound]| {
            bounds
                .iter()
                .map(|bound| bound.to_string())
                .collect::<Vec<String>>()
                .join(", ")
        };

        write!(
            f,
            "BTreeRange: (from: {}, to: {})",
            to_str(&self.0),
            to_str(&self.1)
        )
    }
}
