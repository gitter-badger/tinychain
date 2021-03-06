use std::collections::{HashMap, HashSet};
use std::convert::TryFrom;
use std::fmt;
use std::iter::FromIterator;
use std::ops::Deref;
use std::str::FromStr;

use async_trait::async_trait;
use futures::future::try_join_all;
use log::debug;
use serde::de;
use serde::ser::{Serialize, SerializeSeq, Serializer};

use crate::class::*;
use crate::error;
use crate::general::{Map, Tuple};
use crate::handler::*;
use crate::request::Request;
use crate::transaction::Txn;
use crate::{Match, TCResult, TryCastFrom, TryCastInto};

pub mod map;
pub mod op;
pub mod reference;
pub mod slice;
pub mod value;

pub use op::*;
pub use reference::*;
pub use slice::*;
pub use value::*;

pub trait ScalarInstance: Instance + Sized {
    type Class: ScalarClass;
}

pub trait ScalarClass: Class {
    type Instance: ScalarInstance;

    fn try_cast<S>(&self, scalar: S) -> TCResult<<Self as ScalarClass>::Instance>
    where
        Scalar: From<S>;
}

#[derive(Clone, Eq, PartialEq)]
pub enum ScalarType {
    Map,
    Op(OpDefType),
    Ref(RefType),
    Slice(SliceType),
    Tuple,
    Value(ValueType),
}

impl Class for ScalarType {
    type Instance = Scalar;
}

impl NativeClass for ScalarType {
    fn from_path(path: &[PathSegment]) -> TCResult<Self> {
        let suffix = Self::prefix().try_suffix(path)?;

        if suffix.is_empty() {
            Err(error::method_not_allowed(TCPath::from(path)))
        } else if suffix.len() == 1 {
            match suffix[0].as_str() {
                "map" => Ok(ScalarType::Map),
                "tuple" => Ok(ScalarType::Tuple),
                "op" | "ref" | "slice" | "value" => Err(error::method_not_allowed(&suffix[0])),
                other => Err(error::not_found(other)),
            }
        } else {
            match suffix[0].as_str() {
                "op" => OpDefType::from_path(path).map(ScalarType::Op),
                "ref" => RefType::from_path(path).map(ScalarType::Ref),
                "slice" => SliceType::from_path(path).map(ScalarType::Slice),
                "value" => ValueType::from_path(path).map(ScalarType::Value),
                other => Err(error::not_found(other)),
            }
        }
    }

    fn prefix() -> TCPathBuf {
        TCType::prefix()
    }
}

impl ScalarClass for ScalarType {
    type Instance = Scalar;

    fn try_cast<S>(&self, scalar: S) -> TCResult<Scalar>
    where
        Scalar: From<S>,
    {
        match self {
            Self::Map => match Scalar::from(scalar) {
                Scalar::Map(map) => Ok(Scalar::Map(map)),
                other => Err(error::bad_request("Cannot cast into Map from", other)),
            },
            Self::Op(odt) => odt.try_cast(scalar).map(Box::new).map(Scalar::Op),
            Self::Ref(rt) => rt.try_cast(scalar).map(Box::new).map(Scalar::Ref),
            Self::Slice(st) => st.try_cast(scalar).map(Scalar::Slice),
            Self::Tuple => Scalar::from(scalar)
                .try_cast_into(|v| error::not_implemented(format!("Cast into Tuple from {}", v))),
            Self::Value(vt) => vt.try_cast(scalar).map(Scalar::Value),
        }
    }
}

impl From<ScalarType> for Link {
    fn from(st: ScalarType) -> Link {
        match st {
            ScalarType::Map => ScalarType::prefix().append(label("map")).into(),
            ScalarType::Op(odt) => odt.into(),
            ScalarType::Ref(rt) => rt.into(),
            ScalarType::Slice(st) => st.into(),
            ScalarType::Tuple => ScalarType::prefix().append(label("tuple")).into(),
            ScalarType::Value(vt) => vt.into(),
        }
    }
}

impl From<ScalarType> for TCType {
    fn from(st: ScalarType) -> TCType {
        TCType::Scalar(st)
    }
}

impl fmt::Display for ScalarType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Map => write!(f, "type Scalar Map"),
            Self::Op(odt) => write!(f, "{}", odt),
            Self::Ref(rt) => write!(f, "{}", rt),
            Self::Slice(st) => write!(f, "{}", st),
            Self::Tuple => write!(f, "type Tuple"),
            Self::Value(vt) => write!(f, "{}", vt),
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub enum Scalar {
    Map(Map<Scalar>),
    Op(Box<OpDef>),
    Ref(Box<TCRef>),
    Slice(Slice),
    Tuple(Tuple<Scalar>),
    Value(value::Value),
}

impl Scalar {
    pub fn is_none(&self) -> bool {
        match self {
            Self::Value(value) => value.is_none(),
            Self::Tuple(tuple) => tuple.is_empty(),
            _ => false,
        }
    }

    pub fn is_ref(&self) -> bool {
        match self {
            Self::Map(map) => map.values().any(Scalar::is_ref),
            Self::Ref(_) => true,
            Self::Tuple(tuple) => tuple.iter().any(Scalar::is_ref),
            _ => false,
        }
    }
}

impl Instance for Scalar {
    type Class = ScalarType;

    fn class(&self) -> Self::Class {
        match self {
            Self::Map(_) => ScalarType::Map,
            Self::Op(op) => ScalarType::Op(op.class()),
            Self::Ref(tc_ref) => ScalarType::Ref(tc_ref.class()),
            Self::Slice(slice) => ScalarType::Slice(slice.class()),
            Self::Tuple(_) => ScalarType::Tuple,
            Self::Value(value) => ScalarType::Value(value.class()),
        }
    }
}

#[async_trait]
impl Refer for Scalar {
    fn requires(&self, deps: &mut HashSet<Id>) {
        match self {
            Scalar::Ref(tc_ref) => {
                tc_ref.requires(deps);
            }
            Scalar::Tuple(tuple) => {
                for item in &tuple[..] {
                    item.requires(deps);
                }
            }
            _ => {}
        }
    }

    async fn resolve(
        self,
        request: &Request,
        txn: &Txn,
        context: &HashMap<Id, State>,
    ) -> TCResult<State> {
        match self {
            Scalar::Map(map) => map.resolve(request, txn, context).await,
            Scalar::Ref(tc_ref) => tc_ref.resolve(request, txn, context).await,
            Scalar::Tuple(tuple) => {
                // TODO: use a common impl struct for State::Tuple, Scalar::Tuple, Value::Tuple
                let tuple = try_join_all(
                    tuple
                        .into_inner()
                        .into_iter()
                        .map(|item| item.resolve(request, txn, context)),
                )
                .await?;

                let tuple = Tuple::from(tuple);
                if Scalar::can_cast_from(&tuple) {
                    Ok(State::Scalar(Scalar::opt_cast_from(tuple).unwrap()))
                } else {
                    Ok(State::Tuple(tuple.into()))
                }
            }
            other => Ok(State::Scalar(other)),
        }
    }
}

impl Route for Scalar {
    fn route(&'_ self, method: MethodType, path: &[PathSegment]) -> Option<Box<dyn Handler + '_>> {
        let handler = match self {
            Self::Map(map) => map.route(method, path),
            Self::Op(op) if path.is_empty() => Some(op.handler(None)),
            Self::Value(value) => value.route(method, path),
            _ => None,
        };

        if handler.is_none() && path.is_empty() {
            return Some(Box::new(SelfHandler { scalar: self }));
        } else {
            handler
        }
    }
}

impl ScalarInstance for Scalar {
    type Class = ScalarType;
}

impl From<Number> for Scalar {
    fn from(n: Number) -> Self {
        Scalar::Value(Value::Number(n))
    }
}

impl From<Map<Scalar>> for Scalar {
    fn from(map: Map<Scalar>) -> Self {
        Scalar::Map(map)
    }
}

impl From<Value> for Scalar {
    fn from(value: Value) -> Self {
        Scalar::Value(value)
    }
}

impl From<Id> for Scalar {
    fn from(id: Id) -> Self {
        Scalar::Value(id.into())
    }
}

impl From<()> for Scalar {
    fn from(_: ()) -> Self {
        Scalar::Value(Value::None)
    }
}

impl<T1: Into<Scalar>, T2: Into<Scalar>> From<(T1, T2)> for Scalar {
    fn from(tuple: (T1, T2)) -> Self {
        Scalar::Tuple(vec![tuple.0.into(), tuple.1.into()].into())
    }
}

impl<T: Into<Scalar>> From<Vec<T>> for Scalar {
    fn from(v: Vec<T>) -> Self {
        Scalar::Tuple(v.into_iter().map(|i| i.into()).collect())
    }
}

impl<T: Clone + Into<Scalar>> From<Tuple<T>> for Scalar {
    fn from(v: Tuple<T>) -> Self {
        Self::from_iter(v.into_inner())
    }
}

impl<T: Clone + Into<Scalar>> FromIterator<T> for Scalar {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::Tuple(Tuple::from_iter(iter.into_iter().map(T::into)))
    }
}

impl TryCastFrom<State> for Scalar {
    fn can_cast_from(state: &State) -> bool {
        state.is_scalar()
    }

    fn opt_cast_from(state: State) -> Option<Self> {
        match state {
            State::Scalar(scalar) => Some(scalar),
            _ => None,
        }
    }
}

impl TryCastFrom<Tuple<State>> for Scalar {
    fn can_cast_from(tuple: &Tuple<State>) -> bool {
        tuple.iter().all(State::is_scalar)
    }

    fn opt_cast_from(tuple: Tuple<State>) -> Option<Self> {
        Vec::<Scalar>::opt_cast_from(tuple)
            .map(Tuple::from)
            .map(Scalar::Tuple)
    }
}

impl PartialEq<Value> for Scalar {
    fn eq(&self, that: &Value) -> bool {
        match self {
            Self::Value(this) => this == that,
            _ => false,
        }
    }
}

impl TryFrom<Scalar> for Map<Scalar> {
    type Error = error::TCError;

    fn try_from(s: Scalar) -> TCResult<Self> {
        match s {
            Scalar::Map(map) => Ok(map),
            other => Err(error::bad_request("Expected Scalar Map but found", other)),
        }
    }
}

impl TryFrom<Scalar> for Value {
    type Error = error::TCError;

    fn try_from(s: Scalar) -> TCResult<Value> {
        match s {
            Scalar::Value(value) => Ok(value),
            other => Err(error::bad_request("Expected Value but found", other)),
        }
    }
}

impl TryFrom<Scalar> for Tuple<Scalar> {
    type Error = error::TCError;

    fn try_from(s: Scalar) -> TCResult<Tuple<Scalar>> {
        match s {
            Scalar::Tuple(t) => Ok(t),
            other => Err(error::bad_request("Expected Tuple, found", other)),
        }
    }
}

impl TryCastFrom<Scalar> for Value {
    fn can_cast_from(scalar: &Scalar) -> bool {
        match scalar {
            Scalar::Value(_) => true,
            Scalar::Tuple(tuple) => Value::can_cast_from(tuple),
            _ => false,
        }
    }

    fn opt_cast_from(scalar: Scalar) -> Option<Value> {
        debug!("cast into Value from {}", scalar);

        match scalar {
            Scalar::Value(value) => Some(value),
            Scalar::Tuple(tuple) => Value::opt_cast_from(tuple),
            _ => None,
        }
    }
}

impl TryCastFrom<Scalar> for Link {
    fn can_cast_from(scalar: &Scalar) -> bool {
        if let Scalar::Value(value) = scalar {
            Link::can_cast_from(value)
        } else {
            false
        }
    }

    fn opt_cast_from(scalar: Scalar) -> Option<Link> {
        if let Scalar::Value(value) = scalar {
            Link::opt_cast_from(value)
        } else {
            None
        }
    }
}

impl TryCastFrom<Scalar> for Number {
    fn can_cast_from(scalar: &Scalar) -> bool {
        if let Scalar::Value(value) = scalar {
            Number::can_cast_from(value)
        } else {
            false
        }
    }

    fn opt_cast_from(scalar: Scalar) -> Option<Number> {
        if let Scalar::Value(value) = scalar {
            Number::opt_cast_from(value)
        } else {
            None
        }
    }
}

impl TryCastFrom<Scalar> for Id {
    fn can_cast_from(scalar: &Scalar) -> bool {
        match scalar {
            Scalar::Value(value) => Id::can_cast_from(value),
            Scalar::Ref(tc_ref) => match &**tc_ref {
                TCRef::Id(_) => true,
                _ => false,
            },
            _ => false,
        }
    }

    fn opt_cast_from(scalar: Scalar) -> Option<Id> {
        match scalar {
            Scalar::Value(value) => Id::opt_cast_from(value),
            Scalar::Ref(tc_ref) => match *tc_ref {
                TCRef::Id(id) => Some(id.into()),
                _ => None,
            },
            _ => None,
        }
    }
}

impl<T: TryCastFrom<Scalar>> TryCastFrom<Scalar> for Vec<T> {
    fn can_cast_from(scalar: &Scalar) -> bool {
        if let Scalar::Tuple(values) = scalar {
            Self::can_cast_from(values)
        } else {
            false
        }
    }

    fn opt_cast_from(scalar: Scalar) -> Option<Vec<T>> {
        if let Scalar::Tuple(values) = scalar {
            Self::opt_cast_from(values)
        } else {
            None
        }
    }
}

impl<T: TryCastFrom<Scalar>> TryCastFrom<Scalar> for (T,) {
    fn can_cast_from(source: &Scalar) -> bool {
        if let Scalar::Tuple(source) = source {
            Self::can_cast_from(source)
        } else {
            false
        }
    }

    fn opt_cast_from(source: Scalar) -> Option<(T,)> {
        if let Scalar::Tuple(source) = source {
            Self::opt_cast_from(source)
        } else {
            None
        }
    }
}

impl<T1: TryCastFrom<Scalar>, T2: TryCastFrom<Scalar>> TryCastFrom<Scalar> for (T1, T2) {
    fn can_cast_from(source: &Scalar) -> bool {
        if let Scalar::Tuple(source) = source {
            Self::can_cast_from(source)
        } else {
            false
        }
    }

    fn opt_cast_from(source: Scalar) -> Option<(T1, T2)> {
        if let Scalar::Tuple(source) = source {
            Self::opt_cast_from(source)
        } else {
            None
        }
    }
}

impl<T1: TryCastFrom<Scalar>, T2: TryCastFrom<Scalar>, T3: TryCastFrom<Scalar>> TryCastFrom<Scalar>
    for (T1, T2, T3)
{
    fn can_cast_from(source: &Scalar) -> bool {
        if let Scalar::Tuple(source) = source {
            Self::can_cast_from(source)
        } else {
            false
        }
    }

    fn opt_cast_from(source: Scalar) -> Option<(T1, T2, T3)> {
        if let Scalar::Tuple(source) = source {
            Self::opt_cast_from(source)
        } else {
            None
        }
    }
}

impl<
        T1: TryCastFrom<Scalar>,
        T2: TryCastFrom<Scalar>,
        T3: TryCastFrom<Scalar>,
        T4: TryCastFrom<Scalar>,
    > TryCastFrom<Scalar> for (T1, T2, T3, T4)
{
    fn can_cast_from(source: &Scalar) -> bool {
        if let Scalar::Tuple(source) = source {
            Self::can_cast_from(source)
        } else {
            false
        }
    }

    fn opt_cast_from(source: Scalar) -> Option<(T1, T2, T3, T4)> {
        if let Scalar::Tuple(source) = source {
            Self::opt_cast_from(source)
        } else {
            None
        }
    }
}

struct ScalarVisitor {
    value_visitor: value::ValueVisitor,
}

impl ScalarVisitor {
    fn visit_method<E: de::Error>(
        self,
        subject: IdRef,
        path: value::TCPathBuf,
        params: Scalar,
    ) -> Result<Scalar, E> {
        debug!("Method {} on {}, params {}", path, subject, params);

        let method = if params.is_none() {
            Method::Get((subject, path, Key::Value(Value::None)))
        } else if params.matches::<(Key,)>() {
            let (key,) = params.opt_cast_into().unwrap();
            Method::Get((subject, path, key))
        } else if params.matches::<(Key, Scalar)>() {
            let (key, value) = params.opt_cast_into().unwrap();
            Method::Put((subject, path, key, value))
        } else if params.matches::<Map<Scalar>>() {
            Method::Post((subject, path, params.opt_cast_into().unwrap()))
        } else {
            return Err(de::Error::custom(format!(
                "expected a Method but found {}",
                params
            )));
        };

        Ok(Scalar::Ref(Box::new(TCRef::Method(method))))
    }

    fn visit_op_ref<E: de::Error>(self, link: Link, params: Scalar) -> Result<Scalar, E> {
        debug!("OpRef to {}, params {}", link, params);

        let op_ref = if params.matches::<(Key, Scalar)>() {
            let (key, value) = params.opt_cast_into().unwrap();
            OpRef::Put((link, key, value))
        } else if params.matches::<(Key,)>() {
            let (key,): (Key,) = params.opt_cast_into().unwrap();
            OpRef::Get((link, key))
        } else if params.matches::<Map<Scalar>>() {
            OpRef::Post((link, params.opt_cast_into().unwrap()))
        } else {
            return Err(de::Error::custom(format!("invalid Op format: {}", params)));
        };

        Ok(Scalar::Ref(Box::new(TCRef::Op(op_ref))))
    }
}

impl<'de> de::Visitor<'de> for ScalarVisitor {
    type Value = Scalar;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a Tinychain Scalar, e.g. \"foo\" or 123 or {\"$ref: [\"id\", \"$state\"]\"}")
    }

    fn visit_i16<E: de::Error>(self, value: i16) -> Result<Self::Value, E> {
        self.value_visitor.visit_i16(value).map(Scalar::Value)
    }

    fn visit_i32<E: de::Error>(self, value: i32) -> Result<Self::Value, E> {
        self.value_visitor.visit_i32(value).map(Scalar::Value)
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        self.value_visitor.visit_i64(value).map(Scalar::Value)
    }

    fn visit_u8<E: de::Error>(self, value: u8) -> Result<Self::Value, E> {
        self.value_visitor.visit_u8(value).map(Scalar::Value)
    }

    fn visit_u16<E: de::Error>(self, value: u16) -> Result<Self::Value, E> {
        self.value_visitor.visit_u16(value).map(Scalar::Value)
    }

    fn visit_u32<E: de::Error>(self, value: u32) -> Result<Self::Value, E> {
        self.value_visitor.visit_u32(value).map(Scalar::Value)
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        self.value_visitor.visit_u64(value).map(Scalar::Value)
    }

    fn visit_f32<E: de::Error>(self, value: f32) -> Result<Self::Value, E> {
        self.value_visitor.visit_f32(value).map(Scalar::Value)
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
        self.value_visitor.visit_f64(value).map(Scalar::Value)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        self.value_visitor.visit_str(value).map(Scalar::Value)
    }

    fn visit_seq<L: de::SeqAccess<'de>>(self, mut access: L) -> Result<Self::Value, L::Error> {
        let mut items: Vec<Scalar> = if let Some(size) = access.size_hint() {
            Vec::with_capacity(size)
        } else {
            vec![]
        };

        while let Some(value) = access.next_element()? {
            items.push(value)
        }

        Ok(Scalar::Tuple(items.into()))
    }

    fn visit_map<M: de::MapAccess<'de>>(self, mut access: M) -> Result<Self::Value, M::Error> {
        let mut data: HashMap<String, Scalar> = HashMap::new();

        while let Some(key) = access.next_key()? {
            match access.next_value()? {
                Some(value) => {
                    data.insert(key, value);
                }
                None => {
                    return Err(de::Error::custom(format!(
                        "Failed to parse value of {}",
                        key
                    )))
                }
            }
        }

        if data.is_empty() {
            return Ok(Scalar::Map(Map::default()));
        } else if data.len() == 1 {
            debug!("deserialize map of length 1");
            let (key, data) = data.clone().drain().next().unwrap();

            if key.starts_with('$') {
                debug!("key is a Ref: {}", key);

                let (subject, path) = if let Some(i) = key.find('/') {
                    let (subject, path) = key.split_at(i);
                    let subject = IdRef::from_str(subject).map_err(de::Error::custom)?;
                    let path = TCPathBuf::from_str(path).map_err(de::Error::custom)?;
                    (subject, path)
                } else {
                    (
                        IdRef::from_str(&key).map_err(de::Error::custom)?,
                        TCPathBuf::default(),
                    )
                };

                debug!("{}{} data is {}", subject, path, data);
                return if data.is_none() {
                    if path == TCPathBuf::default() {
                        Ok(Scalar::Ref(Box::new(subject.into())))
                    } else {
                        self.visit_method(subject, path, data)
                    }
                } else {
                    self.visit_method(subject, path, data)
                };
            } else if let Ok(link) = key.parse::<link::Link>() {
                debug!("key is a Link: {}", link);

                if data.is_none() {
                    return Ok(Scalar::Value(Value::TCString(link.into())));
                }

                let path = link.path();
                return if path.len() > 1 && &path[0] == "sbin" {
                    match path[1].as_str() {
                        "value" | "map" | "op" | "ref" | "slice" | "tuple" => match data {
                            Scalar::Value(data) if &path[1] != "slice" => match data {
                                Value::Tuple(tuple) if &path[1] == "tuple" => {
                                    Ok(Scalar::Value(Value::Tuple(tuple)))
                                }
                                Value::Tuple(mut tuple) if tuple.len() == 1 => {
                                    let key = tuple.pop().unwrap();
                                    let dtype = ValueType::from_path(&link.path()[..])
                                        .map_err(de::Error::custom)?;

                                    dtype
                                        .try_cast(key)
                                        .map(Scalar::Value)
                                        .map_err(de::Error::custom)
                                }
                                key => {
                                    let dtype = ValueType::from_path(&link.path()[..])
                                        .map_err(de::Error::custom)?;

                                    dtype
                                        .try_cast(key)
                                        .map(Scalar::Value)
                                        .map_err(de::Error::custom)
                                }
                            },
                            Scalar::Op(_) => {
                                Err(de::Error::custom("{<link>: <op>} does not make sense"))
                            }
                            Scalar::Map(map) if path.len() == 2 && &path[1] == "map" => {
                                Ok(Scalar::Map(map))
                            }
                            Scalar::Map(map) => {
                                Ok(Scalar::Ref(Box::new(TCRef::Op(OpRef::Post((link, map))))))
                            }
                            other => {
                                let dtype = ScalarType::from_path(&link.path()[..])
                                    .map_err(de::Error::custom)?;
                                dtype.try_cast(other).map_err(de::Error::custom)
                            }
                        },
                        _ => self.visit_op_ref::<M::Error>(link, data),
                    }
                } else {
                    self.visit_op_ref::<M::Error>(link, data)
                };
            }
        }

        let mut map = HashMap::with_capacity(data.len());
        for (key, value) in data.drain() {
            debug!("key {} value {}", key, value);
            let key: Id = key.parse().map_err(de::Error::custom)?;
            map.insert(key, value);
        }

        Ok(Scalar::Map(map.into()))
    }
}

impl<'de> de::Deserialize<'de> for Scalar {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value_visitor = value::ValueVisitor;
        d.deserialize_any(ScalarVisitor { value_visitor })
    }
}

impl Serialize for Scalar {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Scalar::Map(map) => map.serialize(s),
            Scalar::Op(op_def) => op_def.serialize(s),
            Scalar::Ref(tc_ref) => tc_ref.serialize(s),
            Scalar::Slice(slice) => slice.serialize(s),
            Scalar::Tuple(tuple) => {
                let mut seq = s.serialize_seq(Some(tuple.len()))?;
                for item in tuple.iter() {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Scalar::Value(value) => value.serialize(s),
        }
    }
}

impl fmt::Display for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Scalar::Map(map) => write!(
                f,
                "{{{}}}",
                map.iter()
                    .map(|(k, v)| format!("{}: {}", k, v))
                    .collect::<Vec<String>>()
                    .join(", ")
            ),
            Scalar::Op(op) => write!(f, "{}", op),
            Scalar::Ref(tc_ref) => write!(f, "{}", tc_ref),
            Scalar::Slice(slice) => write!(f, "{}", slice),
            Scalar::Tuple(tuple) => write!(
                f,
                "[{}]",
                tuple
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<String>>()
                    .join(", ")
            ),
            Scalar::Value(value) => write!(f, "{}", value),
        }
    }
}

struct SelfHandler<'a> {
    scalar: &'a Scalar,
}

#[async_trait]
impl<'a> Handler for SelfHandler<'a> {
    fn subject(&self) -> TCType {
        self.scalar.class().into()
    }

    async fn handle_get(self: Box<Self>, _txn: &Txn, key: Value) -> TCResult<State> {
        if key.is_none() {
            return Ok(State::from(self.scalar.clone()));
        } else if let Scalar::Tuple(tuple) = self.scalar {
            let i: usize =
                key.try_cast_into(|v| error::bad_request("Invalid index for tuple", v))?;

            tuple
                .deref()
                .get(i)
                .cloned()
                .map(State::from)
                .ok_or_else(|| {
                    error::not_found(format!("Index {} in tuple of size {}", i, tuple.len()))
                })
        } else if let Scalar::Value(Value::Tuple(tuple)) = self.scalar {
            let i: usize =
                key.try_cast_into(|v| error::bad_request("Invalid index for tuple", v))?;

            tuple
                .deref()
                .get(i)
                .cloned()
                .map(State::from)
                .ok_or_else(|| {
                    error::not_found(format!("Index {} in tuple of size {}", i, tuple.len()))
                })
        } else {
            Err(error::not_found(format!(
                "{} has no field {}",
                self.scalar.class(),
                key
            )))
        }
    }
}
