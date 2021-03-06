use std::cmp::Ordering;
use std::convert::{TryFrom, TryInto};
use std::fmt;
use std::ops::{Add, Mul, Sub};

use serde::ser::{Serialize, SerializeMap, Serializer};

use crate::class::Instance;
use crate::error;
use crate::handler::{Handler, Route};
use crate::scalar::{Link, MethodType, PathSegment, ScalarInstance, ValueInstance};
use crate::{CastFrom, CastInto, TCResult};

use super::class::{BooleanType, ComplexType, FloatType, IntType, NumberType, UIntType};
use super::class::{NumberClass, NumberInstance};

#[derive(Clone, Copy, PartialEq, PartialOrd)]
pub struct Boolean(bool);

impl Instance for Boolean {
    type Class = BooleanType;

    fn class(&self) -> BooleanType {
        BooleanType
    }
}

impl ScalarInstance for Boolean {
    type Class = BooleanType;
}

impl ValueInstance for Boolean {
    type Class = BooleanType;
}

impl NumberInstance for Boolean {
    type Abs = Self;
    type Class = BooleanType;

    fn into_type(self, _dtype: BooleanType) -> Boolean {
        self
    }

    fn abs(self) -> Self {
        self
    }

    fn and(self, other: Self) -> Self {
        Boolean(self.0 && other.0)
    }

    fn not(self) -> Self {
        Boolean(!self.0)
    }

    fn or(self, other: Self) -> Self {
        Boolean(self.0 || other.0)
    }

    fn xor(self, other: Self) -> Self {
        Boolean(self.0 ^ other.0)
    }
}

impl Route for Boolean {
    fn route(&'_ self, method: MethodType, path: &[PathSegment]) -> Option<Box<dyn Handler + '_>> {
        super::handlers::route(self, method, path)
    }
}

impl Default for Boolean {
    fn default() -> Boolean {
        Boolean(false)
    }
}

impl From<bool> for Boolean {
    fn from(b: bool) -> Boolean {
        Boolean(b)
    }
}

impl From<Boolean> for bool {
    fn from(b: Boolean) -> bool {
        b.0
    }
}

impl From<&Boolean> for bool {
    fn from(b: &Boolean) -> bool {
        b.0
    }
}

impl Eq for Boolean {}

impl Add for Boolean {
    type Output = Self;

    fn add(self, other: Boolean) -> Self::Output {
        match (self, other) {
            (Boolean(false), Boolean(false)) => Boolean(false),
            _ => Boolean(true),
        }
    }
}

impl Mul for Boolean {
    type Output = Self;

    fn mul(self, other: Boolean) -> Self {
        match (self, other) {
            (Boolean(true), Boolean(true)) => Boolean(true),
            _ => Boolean(false),
        }
    }
}

impl Sub for Boolean {
    type Output = Self;

    fn sub(self, other: Boolean) -> Self::Output {
        match (self, other) {
            (left, Boolean(false)) => left,
            _ => Boolean(false),
        }
    }
}

impl CastFrom<Boolean> for u64 {
    fn cast_from(b: Boolean) -> u64 {
        UInt::from(b).into()
    }
}

impl fmt::Display for Boolean {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Serialize for Boolean {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bool(self.0)
    }
}

#[derive(Clone, Copy)]
pub enum Complex {
    C32(num::Complex<f32>),
    C64(num::Complex<f64>),
}

impl Instance for Complex {
    type Class = ComplexType;

    fn class(&self) -> ComplexType {
        match self {
            Complex::C32(_) => ComplexType::C32,
            Complex::C64(_) => ComplexType::C64,
        }
    }
}

impl ScalarInstance for Complex {
    type Class = ComplexType;
}

impl ValueInstance for Complex {
    type Class = ComplexType;
}

impl NumberInstance for Complex {
    type Abs = Float;
    type Class = ComplexType;

    fn into_type(self, dtype: ComplexType) -> Complex {
        use ComplexType::*;
        match dtype {
            C32 => match self {
                Self::C64(c) => Self::C32(num::Complex::new(c.re as f32, c.im as f32)),
                this => this,
            },
            C64 => match self {
                Self::C32(c) => Self::C64(num::Complex::new(c.re as f64, c.im as f64)),
                this => this,
            },
        }
    }

    fn abs(self) -> Float {
        match self {
            Self::C32(c) => Float::F32(c.norm_sqr()),
            Self::C64(c) => Float::F64(c.norm_sqr()),
        }
    }
}

impl Route for Complex {
    fn route(&'_ self, method: MethodType, path: &[PathSegment]) -> Option<Box<dyn Handler + '_>> {
        super::handlers::route(self, method, path)
    }
}

impl CastFrom<Number> for Complex {
    fn cast_from(number: Number) -> Complex {
        use Number::*;
        match number {
            Number::Bool(b) => Self::cast_from(b),
            Complex(c) => c,
            Float(f) => Self::cast_from(f),
            Int(i) => Self::cast_from(i),
            UInt(u) => Self::cast_from(u),
        }
    }
}

impl CastFrom<Complex> for Boolean {
    fn cast_from(c: Complex) -> Self {
        match c {
            Complex::C32(c) if c.norm_sqr() == 0f32 => Boolean(false),
            Complex::C64(c) if c.norm_sqr() == 0f64 => Boolean(false),
            _ => Boolean(true),
        }
    }
}

impl CastFrom<Complex> for num::Complex<f32> {
    fn cast_from(c: Complex) -> Self {
        match c {
            Complex::C32(c) => c,
            Complex::C64(num::Complex { re, im }) => num::Complex::new(re as f32, im as f32),
        }
    }
}

// impl CastFrom<Complex> for num::Complex<f64> {
//     fn cast_from(c: Complex) -> Self {
//         match c {
//             Self::C32(num::Complex { re, im }) => num::Complex::new(re as f64, im as f64),
//             Self::C64(c) => c,
//         }
//     }
// }

impl Add for Complex {
    type Output = Self;

    fn add(self, other: Complex) -> Self {
        match (self, other) {
            (Self::C32(l), Self::C32(r)) => Self::C32(l + r),
            (Self::C64(l), Self::C64(r)) => Self::C64(l + r),
            (Self::C64(l), r) => {
                let r: num::Complex<f64> = r.into();
                Self::C64(l + r)
            }
            (l, r) => r + l,
        }
    }
}

impl Mul for Complex {
    type Output = Self;

    fn mul(self, other: Complex) -> Self {
        match (self, other) {
            (Self::C32(l), Self::C32(r)) => Self::C32(l * r),
            (Self::C64(l), Self::C64(r)) => Self::C64(l * r),
            (Self::C64(l), r) => {
                let r: num::Complex<f64> = r.into();
                Self::C64(l * r)
            }
            (l, r) => r * l,
        }
    }
}

impl Sub for Complex {
    type Output = Self;

    fn sub(self, other: Complex) -> Self {
        match (self, other) {
            (Self::C32(l), Self::C32(r)) => Self::C32(l - r),
            (l, r) => {
                let l: num::Complex<f64> = l.into();
                let r: num::Complex<f64> = r.into();
                Self::C64(l - r)
            }
        }
    }
}

impl PartialEq for Complex {
    fn eq(&self, other: &Self) -> bool {
        type Max = num::complex::Complex<f64>;

        match (self, other) {
            (Self::C32(l), Self::C32(r)) => l.eq(r),
            (Self::C64(l), Self::C64(r)) => l.eq(r),
            (l, r) => Max::from(*l).eq(&Max::from(*r)),
        }
    }
}

impl Eq for Complex {}

impl PartialOrd for Complex {
    fn partial_cmp(&self, other: &Complex) -> Option<Ordering> {
        match (self, other) {
            (Complex::C32(l), Complex::C32(r)) => l.norm_sqr().partial_cmp(&r.norm_sqr()),
            (Complex::C64(l), Complex::C64(r)) => l.norm_sqr().partial_cmp(&r.norm_sqr()),
            _ => None,
        }
    }
}

impl Default for Complex {
    fn default() -> Complex {
        Complex::C32(num::Complex::<f32>::default())
    }
}

impl From<Complex> for num::Complex<f64> {
    fn from(c: Complex) -> Self {
        match c {
            Complex::C32(c) => num::Complex::new(c.re as f64, c.im as f64),
            Complex::C64(c64) => c64,
        }
    }
}

impl From<Float> for Complex {
    fn from(f: Float) -> Self {
        match f {
            Float::F64(f) => Self::C64(num::Complex::new(f, 0.0f64)),
            Float::F32(f) => Self::C32(num::Complex::new(f, 0.0f32)),
        }
    }
}

impl From<Int> for Complex {
    fn from(i: Int) -> Self {
        match i {
            Int::I64(i) => Self::C64(num::Complex::new(i as f64, 0.0f64)),
            Int::I32(i) => Self::C32(num::Complex::new(i as f32, 0.0f32)),
            Int::I16(i) => Self::C32(num::Complex::new(i as f32, 0.0f32)),
        }
    }
}

impl From<UInt> for Complex {
    fn from(u: UInt) -> Self {
        match u {
            UInt::U64(u) => Self::C64(num::Complex::new(u as f64, 0.0f64)),
            UInt::U32(u) => Self::C32(num::Complex::new(u as f32, 0.0f32)),
            UInt::U16(u) => Self::C32(num::Complex::new(u as f32, 0.0f32)),
            UInt::U8(u) => Self::C32(num::Complex::new(u as f32, 0.0f32)),
        }
    }
}

impl From<Boolean> for Complex {
    fn from(b: Boolean) -> Self {
        match b {
            Boolean(true) => Self::C32(num::Complex::new(1.0f32, 0.0f32)),
            Boolean(false) => Self::C32(num::Complex::new(1.0f32, 0.0f32)),
        }
    }
}

impl From<num::Complex<f32>> for Complex {
    fn from(c: num::Complex<f32>) -> Complex {
        Complex::C32(c)
    }
}

impl From<num::Complex<f64>> for Complex {
    fn from(c: num::Complex<f64>) -> Complex {
        Complex::C64(c)
    }
}

impl TryFrom<Complex> for num::Complex<f32> {
    type Error = error::TCError;

    fn try_from(c: Complex) -> TCResult<num::Complex<f32>> {
        match c {
            Complex::C32(c) => Ok(c),
            other => Err(error::bad_request("Expected C32 but found", other)),
        }
    }
}

impl Serialize for Complex {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Complex::C32(c) => {
                let mut map = s.serialize_map(Some(1))?;
                map.serialize_entry(&Link::from(self.class()).to_string(), &[c.re, c.im])?;
                map.end()
            }
            Complex::C64(c) => {
                let mut map = s.serialize_map(Some(1))?;
                map.serialize_entry(&Link::from(self.class()).to_string(), &[c.re, c.im])?;
                map.end()
            }
        }
    }
}

impl fmt::Display for Complex {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Complex::C32(c) => write!(f, "C32({})", c),
            Complex::C64(c) => write!(f, "C64({})", c),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Float {
    F32(f32),
    F64(f64),
}

impl Instance for Float {
    type Class = FloatType;

    fn class(&self) -> FloatType {
        match self {
            Float::F32(_) => FloatType::F32,
            Float::F64(_) => FloatType::F64,
        }
    }
}

impl ScalarInstance for Float {
    type Class = FloatType;
}

impl ValueInstance for Float {
    type Class = FloatType;
}

impl NumberInstance for Float {
    type Abs = Float;
    type Class = FloatType;

    fn into_type(self, dtype: FloatType) -> Float {
        use FloatType::*;
        match dtype {
            F32 => match self {
                Self::F64(f) => Self::F32(f as f32),
                this => this,
            },
            F64 => match self {
                Self::F32(f) => Self::F64(f as f64),
                this => this,
            },
        }
    }

    fn abs(self) -> Float {
        match self {
            Self::F32(f) => Self::F32(f.abs()),
            Self::F64(f) => Self::F64(f.abs()),
        }
    }
}

impl Route for Float {
    fn route(&'_ self, method: MethodType, path: &[PathSegment]) -> Option<Box<dyn Handler + '_>> {
        super::handlers::route(self, method, path)
    }
}

impl CastFrom<Complex> for Float {
    fn cast_from(c: Complex) -> Float {
        use Complex::*;
        match c {
            C32(c) => Self::F32(c.re),
            C64(c) => Self::F64(c.re),
        }
    }
}

impl CastFrom<Float> for Boolean {
    fn cast_from(f: Float) -> Boolean {
        use Float::*;
        let b = match f {
            F32(f) if f == 0f32 => false,
            F64(f) if f == 0f64 => false,
            _ => true,
        };

        Boolean(b)
    }
}

impl CastFrom<Float> for f32 {
    fn cast_from(f: Float) -> f32 {
        match f {
            Float::F32(f) => f,
            Float::F64(f) => f as f32,
        }
    }
}

impl Eq for Float {}

impl Add for Float {
    type Output = Self;

    fn add(self, other: Float) -> Self {
        match (self, other) {
            (Self::F32(l), Self::F32(r)) => Self::F32(l + r),
            (Self::F64(l), Self::F64(r)) => Self::F64(l + r),
            (Self::F64(l), Self::F32(r)) => Self::F64(l + r as f64),
            (l, r) => (r + l),
        }
    }
}

impl Mul for Float {
    type Output = Self;

    fn mul(self, other: Float) -> Self {
        match (self, other) {
            (Self::F32(l), Self::F32(r)) => Self::F32(l * r),
            (Self::F64(l), Self::F64(r)) => Self::F64(l * r),
            (Self::F64(l), Self::F32(r)) => Self::F64(l * r as f64),
            (l, r) => (r * l),
        }
    }
}

impl Sub for Float {
    type Output = Self;

    fn sub(self, other: Float) -> Self {
        match (self, other) {
            (Self::F32(l), Self::F32(r)) => Self::F32(l - r),
            (l, r) => {
                let l: f64 = l.into();
                let r: f64 = r.into();
                Self::F64(l - r)
            }
        }
    }
}

impl PartialEq for Float {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::F32(l), Self::F32(r)) => l.eq(r),
            (Self::F64(l), Self::F64(r)) => l.eq(r),
            (l, r) => f64::from(*l).eq(&f64::from(*r)),
        }
    }
}

impl PartialOrd for Float {
    fn partial_cmp(&self, other: &Float) -> Option<Ordering> {
        match (self, other) {
            (Float::F32(l), Float::F32(r)) => l.partial_cmp(r),
            (Float::F64(l), Float::F64(r)) => l.partial_cmp(r),
            _ => None,
        }
    }
}

impl Default for Float {
    fn default() -> Float {
        Float::F32(f32::default())
    }
}

impl From<Boolean> for Float {
    fn from(b: Boolean) -> Self {
        match b {
            Boolean(true) => Self::F32(1.0f32),
            Boolean(false) => Self::F32(0.0f32),
        }
    }
}

impl From<f32> for Float {
    fn from(f: f32) -> Self {
        Self::F32(f)
    }
}

impl From<f64> for Float {
    fn from(f: f64) -> Self {
        Self::F64(f)
    }
}

impl From<Int> for Float {
    fn from(i: Int) -> Self {
        match i {
            Int::I64(i) => Self::F64(i as f64),
            Int::I32(i) => Self::F32(i as f32),
            Int::I16(i) => Self::F32(i as f32),
        }
    }
}

impl From<UInt> for Float {
    fn from(u: UInt) -> Self {
        match u {
            UInt::U64(u) => Self::F64(u as f64),
            UInt::U32(u) => Self::F32(u as f32),
            UInt::U16(u) => Self::F32(u as f32),
            UInt::U8(u) => Self::F32(u as f32),
        }
    }
}

impl TryFrom<Float> for f32 {
    type Error = error::TCError;

    fn try_from(f: Float) -> TCResult<f32> {
        match f {
            Float::F32(f) => Ok(f),
            other => Err(error::bad_request("Expected F32 but found", other)),
        }
    }
}

impl From<Float> for f64 {
    fn from(f: Float) -> f64 {
        match f {
            Float::F32(f) => f as f64,
            Float::F64(f) => f,
        }
    }
}

impl Serialize for Float {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Float::F32(f) => s.serialize_f32(*f),
            Float::F64(f) => s.serialize_f64(*f),
        }
    }
}

impl fmt::Display for Float {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Float::F32(i) => write!(f, "F32({})", i),
            Float::F64(i) => write!(f, "F64({})", i),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Int {
    I16(i16),
    I32(i32),
    I64(i64),
}

impl Instance for Int {
    type Class = IntType;

    fn class(&self) -> IntType {
        match self {
            Int::I16(_) => IntType::I16,
            Int::I32(_) => IntType::I32,
            Int::I64(_) => IntType::I64,
        }
    }
}

impl ScalarInstance for Int {
    type Class = IntType;
}

impl ValueInstance for Int {
    type Class = IntType;
}

impl NumberInstance for Int {
    type Abs = Self;
    type Class = IntType;

    fn into_type(self, dtype: IntType) -> Int {
        use IntType::*;
        match dtype {
            I16 => match self {
                Self::I32(i) => Self::I16(i as i16),
                Self::I64(i) => Self::I16(i as i16),
                this => this,
            },
            I32 => match self {
                Self::I16(i) => Self::I32(i as i32),
                Self::I64(i) => Self::I32(i as i32),
                this => this,
            },
            I64 => match self {
                Self::I16(i) => Self::I64(i as i64),
                Self::I32(i) => Self::I64(i as i64),
                this => this,
            },
        }
    }

    fn abs(self) -> Self {
        match self {
            Self::I16(i) => Int::I16(i.abs()),
            Self::I32(i) => Int::I32(i.abs()),
            Self::I64(i) => Int::I64(i.abs()),
        }
    }
}

impl Route for Int {
    fn route(&'_ self, method: MethodType, path: &[PathSegment]) -> Option<Box<dyn Handler + '_>> {
        super::handlers::route(self, method, path)
    }
}

impl CastFrom<Complex> for Int {
    fn cast_from(c: Complex) -> Int {
        use Complex::*;
        match c {
            C32(c) => Self::I32(c.re as i32),
            C64(c) => Self::I64(c.re as i64),
        }
    }
}

impl CastFrom<Float> for Int {
    fn cast_from(f: Float) -> Int {
        use Float::*;
        match f {
            F32(f) => Self::I32(f as i32),
            F64(f) => Self::I64(f as i64),
        }
    }
}

impl CastFrom<Int> for Boolean {
    fn cast_from(i: Int) -> Boolean {
        use Int::*;
        let b = match i {
            I16(i) if i == 0i16 => false,
            I32(i) if i == 0i32 => false,
            I64(i) if i == 0i64 => false,
            _ => true,
        };

        Boolean(b)
    }
}

impl CastFrom<Int> for i16 {
    fn cast_from(i: Int) -> i16 {
        match i {
            Int::I16(i) => i,
            Int::I32(i) => i as i16,
            Int::I64(i) => i as i16,
        }
    }
}

impl CastFrom<Int> for i32 {
    fn cast_from(i: Int) -> i32 {
        match i {
            Int::I16(i) => i as i32,
            Int::I32(i) => i,
            Int::I64(i) => i as i32,
        }
    }
}

impl Eq for Int {}

impl Add for Int {
    type Output = Self;

    fn add(self, other: Int) -> Self {
        match (self, other) {
            (Self::I64(l), Self::I64(r)) => Self::I64(l + r),
            (Self::I64(l), Self::I32(r)) => Self::I64(l + r as i64),
            (Self::I64(l), Self::I16(r)) => Self::I64(l + r as i64),
            (Self::I32(l), Self::I32(r)) => Self::I32(l + r),
            (Self::I32(l), Self::I16(r)) => Self::I32(l + r as i32),
            (Self::I16(l), Self::I16(r)) => Self::I16(l + r),
            (l, r) => r + l,
        }
    }
}

impl Mul for Int {
    type Output = Self;

    fn mul(self, other: Int) -> Self {
        match (self, other) {
            (Self::I64(l), Self::I64(r)) => Self::I64(l * r),
            (Self::I64(l), Self::I32(r)) => Self::I64(l * r as i64),
            (Self::I64(l), Self::I16(r)) => Self::I64(l * r as i64),
            (Self::I32(l), Self::I32(r)) => Self::I32(l * r),
            (Self::I32(l), Self::I16(r)) => Self::I32(l * r as i32),
            (Self::I16(l), Self::I16(r)) => Self::I16(l * r),
            (l, r) => r * l,
        }
    }
}

impl Sub for Int {
    type Output = Self;

    fn sub(self, other: Int) -> Self {
        match (self, other) {
            (Self::I64(l), Self::I64(r)) => Self::I64(l - r),
            (Self::I64(l), Self::I32(r)) => Self::I64(l - r as i64),
            (Self::I64(l), Self::I16(r)) => Self::I64(l - r as i64),
            (Self::I32(l), Self::I32(r)) => Self::I32(l - r),
            (Self::I32(l), Self::I16(r)) => Self::I32(l - r as i32),
            (Self::I16(l), Self::I16(r)) => Self::I16(l - r),
            (Self::I16(l), Self::I32(r)) => Self::I32(l as i32 - r),
            (Self::I16(l), Self::I64(r)) => Self::I64(l as i64 - r),
            (Self::I32(l), Self::I64(r)) => Self::I64(l as i64 - r),
        }
    }
}

impl PartialEq for Int {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::I16(l), Self::I16(r)) => l.eq(r),
            (Self::I32(l), Self::I32(r)) => l.eq(r),
            (Self::I64(l), Self::I64(r)) => l.eq(r),
            (Self::I64(l), r) => l.eq(&i64::from(*r)),
            (l, r) => i64::from(*l).eq(&i64::from(*r)),
        }
    }
}

impl PartialOrd for Int {
    fn partial_cmp(&self, other: &Int) -> Option<Ordering> {
        match (self, other) {
            (Int::I16(l), Int::I16(r)) => l.partial_cmp(r),
            (Int::I32(l), Int::I32(r)) => l.partial_cmp(r),
            (Int::I64(l), Int::I64(r)) => l.partial_cmp(r),
            _ => None,
        }
    }
}

impl Default for Int {
    fn default() -> Int {
        Int::I16(i16::default())
    }
}

impl From<i16> for Int {
    fn from(i: i16) -> Int {
        Int::I16(i)
    }
}

impl From<i32> for Int {
    fn from(i: i32) -> Int {
        Int::I32(i)
    }
}

impl From<i64> for Int {
    fn from(i: i64) -> Int {
        Int::I64(i)
    }
}

impl From<UInt> for Int {
    fn from(u: UInt) -> Int {
        match u {
            UInt::U64(u) => Int::I64(u as i64),
            UInt::U32(u) => Int::I32(u as i32),
            UInt::U16(u) => Int::I16(u as i16),
            UInt::U8(u) => Int::I16(u as i16),
        }
    }
}

impl From<Boolean> for Int {
    fn from(b: Boolean) -> Int {
        match b {
            Boolean(true) => Int::I16(1),
            Boolean(false) => Int::I16(0),
        }
    }
}

impl TryFrom<Int> for i16 {
    type Error = error::TCError;

    fn try_from(i: Int) -> TCResult<i16> {
        match i {
            Int::I16(i) => Ok(i),
            other => Err(error::bad_request("Expected I16 but found", other)),
        }
    }
}

impl TryFrom<Int> for i32 {
    type Error = error::TCError;

    fn try_from(i: Int) -> TCResult<i32> {
        match i {
            Int::I32(i) => Ok(i),
            other => Err(error::bad_request("Expected I32 but found", other)),
        }
    }
}

impl From<Int> for i64 {
    fn from(i: Int) -> i64 {
        match i {
            Int::I16(i) => i as i64,
            Int::I32(i) => i as i64,
            Int::I64(i) => i,
        }
    }
}

impl Serialize for Int {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Int::I16(i) => s.serialize_i16(*i),
            Int::I32(i) => s.serialize_i32(*i),
            Int::I64(i) => s.serialize_i64(*i),
        }
    }
}

impl fmt::Display for Int {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Int::I16(i) => write!(f, "I16: {}", i),
            Int::I32(i) => write!(f, "I32: {}", i),
            Int::I64(i) => write!(f, "I64: {}", i),
        }
    }
}

#[derive(Clone, Copy)]
pub enum UInt {
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
}

impl Instance for UInt {
    type Class = UIntType;

    fn class(&self) -> UIntType {
        match self {
            UInt::U8(_) => UIntType::U8,
            UInt::U16(_) => UIntType::U16,
            UInt::U32(_) => UIntType::U32,
            UInt::U64(_) => UIntType::U64,
        }
    }
}

impl ScalarInstance for UInt {
    type Class = UIntType;
}

impl ValueInstance for UInt {
    type Class = UIntType;
}

impl NumberInstance for UInt {
    type Abs = Self;
    type Class = UIntType;

    fn into_type(self, dtype: UIntType) -> UInt {
        use UIntType::*;
        match dtype {
            U8 => match self {
                Self::U16(u) => Self::U8(u as u8),
                Self::U32(u) => Self::U8(u as u8),
                Self::U64(u) => Self::U8(u as u8),
                this => this,
            },
            U16 => match self {
                Self::U8(u) => Self::U16(u as u16),
                Self::U32(u) => Self::U16(u as u16),
                Self::U64(u) => Self::U16(u as u16),
                this => this,
            },
            U32 => match self {
                Self::U8(u) => Self::U32(u as u32),
                Self::U16(u) => Self::U32(u as u32),
                Self::U64(u) => Self::U32(u as u32),
                this => this,
            },
            U64 => match self {
                Self::U8(u) => Self::U64(u as u64),
                Self::U16(u) => Self::U64(u as u64),
                Self::U32(u) => Self::U64(u as u64),
                this => this,
            },
        }
    }

    fn abs(self) -> UInt {
        self
    }
}

impl Route for UInt {
    fn route(&'_ self, method: MethodType, path: &[PathSegment]) -> Option<Box<dyn Handler + '_>> {
        super::handlers::route(self, method, path)
    }
}

impl CastFrom<Complex> for UInt {
    fn cast_from(c: Complex) -> UInt {
        use Complex::*;
        match c {
            C32(c) => Self::U32(c.re as u32),
            C64(c) => Self::U64(c.re as u64),
        }
    }
}

impl CastFrom<Float> for UInt {
    fn cast_from(f: Float) -> UInt {
        use Float::*;
        match f {
            F32(f) => Self::U32(f as u32),
            F64(f) => Self::U64(f as u64),
        }
    }
}

impl CastFrom<Int> for UInt {
    fn cast_from(i: Int) -> UInt {
        use Int::*;
        match i {
            I16(i) => Self::U16(i as u16),
            I32(i) => Self::U32(i as u32),
            I64(i) => Self::U64(i as u64),
        }
    }
}

impl CastFrom<UInt> for bool {
    fn cast_from(u: UInt) -> bool {
        use UInt::*;
        match u {
            U8(u) if u == 0u8 => false,
            U16(u) if u == 0u16 => false,
            U32(u) if u == 0u32 => false,
            U64(u) if u == 0u64 => false,
            _ => true,
        }
    }
}

impl CastFrom<UInt> for u8 {
    fn cast_from(u: UInt) -> u8 {
        use UInt::*;
        match u {
            U8(u) => u,
            U16(u) => u as u8,
            U32(u) => u as u8,
            U64(u) => u as u8,
        }
    }
}

impl CastFrom<UInt> for u16 {
    fn cast_from(u: UInt) -> u16 {
        use UInt::*;
        match u {
            U8(u) => u as u16,
            U16(u) => u,
            U32(u) => u as u16,
            U64(u) => u as u16,
        }
    }
}

impl CastFrom<UInt> for u32 {
    fn cast_from(u: UInt) -> u32 {
        use UInt::*;
        match u {
            U8(u) => u as u32,
            U16(u) => u as u32,
            U32(u) => u,
            U64(u) => u as u32,
        }
    }
}

impl Add for UInt {
    type Output = Self;

    fn add(self, other: UInt) -> Self {
        match (self, other) {
            (UInt::U64(l), UInt::U64(r)) => UInt::U64(l + r),
            (UInt::U64(l), UInt::U32(r)) => UInt::U64(l + r as u64),
            (UInt::U64(l), UInt::U16(r)) => UInt::U64(l + r as u64),
            (UInt::U64(l), UInt::U8(r)) => UInt::U64(l + r as u64),
            (UInt::U32(l), UInt::U32(r)) => UInt::U32(l + r),
            (UInt::U32(l), UInt::U16(r)) => UInt::U32(l + r as u32),
            (UInt::U32(l), UInt::U8(r)) => UInt::U32(l + r as u32),
            (UInt::U16(l), UInt::U16(r)) => UInt::U16(l + r),
            (UInt::U16(l), UInt::U8(r)) => UInt::U16(l + r as u16),
            (UInt::U8(l), UInt::U8(r)) => UInt::U8(l + r),
            (l, r) => r + l,
        }
    }
}

impl Mul for UInt {
    type Output = Self;

    fn mul(self, other: UInt) -> Self {
        match (self, other) {
            (UInt::U64(l), UInt::U64(r)) => UInt::U64(l * r),
            (UInt::U64(l), UInt::U32(r)) => UInt::U64(l * r as u64),
            (UInt::U64(l), UInt::U16(r)) => UInt::U64(l * r as u64),
            (UInt::U64(l), UInt::U8(r)) => UInt::U64(l * r as u64),
            (UInt::U32(l), UInt::U32(r)) => UInt::U32(l * r),
            (UInt::U32(l), UInt::U16(r)) => UInt::U32(l * r as u32),
            (UInt::U32(l), UInt::U8(r)) => UInt::U32(l * r as u32),
            (UInt::U16(l), UInt::U16(r)) => UInt::U16(l * r),
            (UInt::U16(l), UInt::U8(r)) => UInt::U16(l * r as u16),
            (UInt::U8(l), UInt::U8(r)) => UInt::U8(l * r),
            (l, r) => r * l,
        }
    }
}

impl Sub for UInt {
    type Output = Self;

    fn sub(self, other: UInt) -> Self {
        match (self, other) {
            (UInt::U64(l), UInt::U64(r)) => UInt::U64(l - r),
            (UInt::U64(l), UInt::U32(r)) => UInt::U64(l - r as u64),
            (UInt::U64(l), UInt::U16(r)) => UInt::U64(l - r as u64),
            (UInt::U64(l), UInt::U8(r)) => UInt::U64(l - r as u64),
            (UInt::U32(l), UInt::U32(r)) => UInt::U32(l - r),
            (UInt::U32(l), UInt::U16(r)) => UInt::U32(l - r as u32),
            (UInt::U32(l), UInt::U8(r)) => UInt::U32(l - r as u32),
            (UInt::U16(l), UInt::U16(r)) => UInt::U16(l - r),
            (UInt::U16(l), UInt::U8(r)) => UInt::U16(l - r as u16),
            (UInt::U8(l), UInt::U8(r)) => UInt::U8(l - r),
            (UInt::U8(l), UInt::U16(r)) => UInt::U16(l as u16 - r),
            (UInt::U8(l), UInt::U32(r)) => UInt::U32(l as u32 - r),
            (UInt::U8(l), UInt::U64(r)) => UInt::U64(l as u64 - r),
            (UInt::U16(l), r) => {
                let r: u64 = r.into();
                UInt::U16(l - r as u16)
            }
            (UInt::U32(l), r) => {
                let r: u64 = r.into();
                UInt::U32(l - r as u32)
            }
        }
    }
}

impl Eq for UInt {}

impl Ord for UInt {
    fn cmp(&self, other: &UInt) -> Ordering {
        match (self, other) {
            (UInt::U64(l), UInt::U64(r)) => l.cmp(r),
            (UInt::U64(l), UInt::U32(r)) => l.cmp(&r.clone().into()),
            (UInt::U64(l), UInt::U16(r)) => l.cmp(&r.clone().into()),
            (UInt::U64(l), UInt::U8(r)) => l.cmp(&r.clone().into()),
            (UInt::U32(l), UInt::U32(r)) => l.cmp(r),
            (UInt::U32(l), UInt::U16(r)) => l.cmp(&r.clone().into()),
            (UInt::U32(l), UInt::U8(r)) => l.cmp(&r.clone().into()),
            (UInt::U16(l), UInt::U16(r)) => l.cmp(r),
            (UInt::U16(l), UInt::U8(r)) => l.cmp(&r.clone().into()),
            (UInt::U8(l), UInt::U8(r)) => l.cmp(r),
            (l, r) => match r.cmp(l) {
                Ordering::Greater => Ordering::Less,
                Ordering::Less => Ordering::Greater,
                Ordering::Equal => Ordering::Equal,
            },
        }
    }
}

impl PartialEq for UInt {
    fn eq(&self, other: &UInt) -> bool {
        match (self, other) {
            (Self::U8(l), Self::U8(r)) => l.eq(r),
            (Self::U16(l), Self::U16(r)) => l.eq(r),
            (Self::U32(l), Self::U32(r)) => l.eq(r),
            (Self::U64(l), Self::U64(r)) => l.eq(r),
            (l, r) => u64::from(*l).eq(&u64::from(*r)),
        }
    }
}

impl PartialOrd for UInt {
    fn partial_cmp(&self, other: &UInt) -> Option<Ordering> {
        match (self, other) {
            (Self::U8(l), Self::U8(r)) => l.partial_cmp(r),
            (Self::U16(l), Self::U16(r)) => l.partial_cmp(r),
            (Self::U32(l), Self::U32(r)) => l.partial_cmp(r),
            (Self::U64(l), Self::U64(r)) => l.partial_cmp(r),
            (l, r) => u64::from(*l).partial_cmp(&u64::from(*r)),
        }
    }
}

impl Default for UInt {
    fn default() -> UInt {
        UInt::U8(u8::default())
    }
}

impl From<Boolean> for UInt {
    fn from(b: Boolean) -> UInt {
        match b {
            Boolean(true) => UInt::U8(1),
            Boolean(false) => UInt::U8(0),
        }
    }
}

impl From<u8> for UInt {
    fn from(u: u8) -> UInt {
        UInt::U8(u)
    }
}

impl From<u16> for UInt {
    fn from(u: u16) -> UInt {
        UInt::U16(u)
    }
}

impl From<u32> for UInt {
    fn from(u: u32) -> UInt {
        UInt::U32(u)
    }
}

impl From<u64> for UInt {
    fn from(u: u64) -> UInt {
        UInt::U64(u)
    }
}

impl TryFrom<UInt> for u8 {
    type Error = error::TCError;

    fn try_from(u: UInt) -> TCResult<u8> {
        match u {
            UInt::U8(u) => Ok(u),
            other => Err(error::bad_request("Expected a UInt8 but found", other)),
        }
    }
}

impl TryFrom<UInt> for u16 {
    type Error = error::TCError;

    fn try_from(u: UInt) -> TCResult<u16> {
        match u {
            UInt::U16(u) => Ok(u),
            other => Err(error::bad_request("Expected a UInt16 but found", other)),
        }
    }
}

impl TryFrom<UInt> for u32 {
    type Error = error::TCError;

    fn try_from(u: UInt) -> TCResult<u32> {
        match u {
            UInt::U32(u) => Ok(u),
            other => Err(error::bad_request("Expected a UInt32 but found", other)),
        }
    }
}

impl From<UInt> for u64 {
    fn from(u: UInt) -> u64 {
        match u {
            UInt::U64(u) => u,
            UInt::U32(u) => u as u64,
            UInt::U16(u) => u as u64,
            UInt::U8(u) => u as u64,
        }
    }
}

impl From<UInt> for usize {
    fn from(u: UInt) -> usize {
        match u {
            UInt::U64(u) => u as usize,
            UInt::U32(u) => u as usize,
            UInt::U16(u) => u as usize,
            UInt::U8(u) => u as usize,
        }
    }
}

impl Serialize for UInt {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            UInt::U8(u) => s.serialize_u8(*u),
            UInt::U16(u) => s.serialize_u16(*u),
            UInt::U32(u) => s.serialize_u32(*u),
            UInt::U64(u) => s.serialize_u64(*u),
        }
    }
}

impl fmt::Display for UInt {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            UInt::U8(u) => write!(f, "U8: {}", u),
            UInt::U16(u) => write!(f, "UInt16: {}", u),
            UInt::U32(u) => write!(f, "UInt32: {}", u),
            UInt::U64(u) => write!(f, "UInt64: {}", u),
        }
    }
}

#[derive(Clone, Copy, Eq)]
pub enum Number {
    Bool(Boolean),
    Complex(Complex),
    Float(Float),
    Int(Int),
    UInt(UInt),
}

impl Instance for Number {
    type Class = NumberType;

    fn class(&self) -> NumberType {
        use NumberType::*;
        match self {
            Self::Bool(_) => Bool,
            Self::Complex(c) => Complex(c.class()),
            Self::Float(f) => Float(f.class()),
            Self::Int(i) => Int(i.class()),
            Self::UInt(u) => UInt(u.class()),
        }
    }
}

impl ScalarInstance for Number {
    type Class = NumberType;
}

impl ValueInstance for Number {
    type Class = NumberType;
}

impl NumberInstance for Number {
    type Abs = Number;
    type Class = NumberType;

    fn into_type(self, dtype: NumberType) -> Number {
        use NumberType as NT;

        match dtype {
            NT::Bool => {
                let b: Boolean = self.cast_into();
                b.into()
            }
            NT::Complex(ct) => {
                let c: Complex = self.cast_into();
                c.into_type(ct).into()
            }
            NT::Float(ft) => {
                let f: Float = self.cast_into();
                f.into_type(ft).into()
            }
            NT::Int(it) => {
                let i: Int = self.cast_into();
                i.into_type(it).into()
            }
            NT::UInt(ut) => {
                let u: UInt = self.cast_into();
                u.into_type(ut).into()
            }
            NT::Number => self,
        }
    }

    fn abs(self) -> Number {
        use Number::*;
        match self {
            Complex(c) => Float(c.abs()),
            Float(f) => Float(f.abs()),
            Int(i) => Int(i.abs()),
            other => other,
        }
    }
}

impl Route for Number {
    fn route(&'_ self, method: MethodType, path: &[PathSegment]) -> Option<Box<dyn Handler + '_>> {
        super::handlers::route(self, method, path)
    }
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Int(l), Self::Int(r)) => l.eq(r),
            (Self::UInt(l), Self::UInt(r)) => l.eq(r),
            (Self::Float(l), Self::Float(r)) => l.eq(r),
            (Self::Bool(l), Self::Bool(r)) => l.eq(r),
            (Self::Complex(l), Self::Complex(r)) => l.eq(r),

            (Self::Complex(l), r) => l.eq(&Complex::cast_from(*r)),
            (Self::Float(l), r) => l.eq(&Float::cast_from(*r)),
            (Self::Int(l), r) => l.eq(&Int::cast_from(*r)),
            (Self::UInt(l), r) => l.eq(&UInt::cast_from(*r)),

            (l, r) => r.eq(l),
        }
    }
}

impl PartialOrd for Number {
    fn partial_cmp(&self, other: &Number) -> Option<Ordering> {
        match (self, other) {
            (Self::Int(l), Self::Int(r)) => l.partial_cmp(r),
            (Self::UInt(l), Self::UInt(r)) => l.partial_cmp(r),
            (Self::Float(l), Self::Float(r)) => l.partial_cmp(r),
            (Self::Bool(l), Self::Bool(r)) => l.partial_cmp(r),
            (Self::Complex(l), Self::Complex(r)) => l.partial_cmp(r),

            (Self::Complex(l), r) => l.partial_cmp(&Complex::cast_from(*r)),
            (Self::Float(l), r) => l.partial_cmp(&Float::cast_from(*r)),
            (Self::Int(l), r) => l.partial_cmp(&Int::cast_from(*r)),
            (Self::UInt(l), r) => l.partial_cmp(&UInt::cast_from(*r)),

            (l, r) => match r.partial_cmp(l) {
                Some(ordering) => Some(match ordering {
                    Ordering::Less => Ordering::Greater,
                    Ordering::Equal => Ordering::Equal,
                    Ordering::Greater => Ordering::Less,
                }),
                None => None,
            },
        }
    }
}

impl Add for Number {
    type Output = Self;

    fn add(self, other: Number) -> Self {
        let dtype = Ord::max(self.class(), other.class());

        use NumberType as NT;

        match dtype {
            NT::Bool => {
                let this: Boolean = self.cast_into();
                (this + other.cast_into()).into()
            }
            NT::Complex(_) => {
                let this: Complex = self.cast_into();
                (this + other.cast_into()).into()
            }
            NT::Float(_) => {
                let this: Float = self.cast_into();
                (this + other.cast_into()).into()
            }
            NT::Int(_) => {
                let this: Int = self.cast_into();
                (this + other.cast_into()).into()
            }
            NT::UInt(_) => {
                let this: UInt = self.cast_into();
                (this + other.cast_into()).into()
            }
            NT::Number => panic!("A number instance must have a specific type, not Number"),
        }
    }
}

impl Mul for Number {
    type Output = Self;

    fn mul(self, other: Number) -> Self {
        let dtype = Ord::max(self.class(), other.class());

        use NumberType as NT;

        match dtype {
            NT::Bool => {
                let this: Boolean = self.cast_into();
                (this * other.cast_into()).into()
            }
            NT::Complex(_) => {
                let this: Complex = self.cast_into();
                (this * other.cast_into()).into()
            }
            NT::Float(_) => {
                let this: Float = self.cast_into();
                (this * other.cast_into()).into()
            }
            NT::Int(_) => {
                let this: Int = self.cast_into();
                (this * other.cast_into()).into()
            }
            NT::UInt(_) => {
                let this: UInt = self.cast_into();
                (this * other.cast_into()).into()
            }
            NT::Number => panic!("A number instance must have a specific type, not Number"),
        }
    }
}

impl Sub for Number {
    type Output = Self;

    fn sub(self, other: Number) -> Self {
        let dtype = Ord::max(self.class(), other.class());

        use NumberType as NT;

        match dtype {
            NT::Bool => {
                let this: Boolean = self.cast_into();
                (this - other.cast_into()).into()
            }
            NT::Complex(_) => {
                let this: Complex = self.cast_into();
                (this - other.cast_into()).into()
            }
            NT::Float(_) => {
                let this: Float = self.cast_into();
                (this - other.cast_into()).into()
            }
            NT::Int(_) => {
                let this: Int = self.cast_into();
                (this - other.cast_into()).into()
            }
            NT::UInt(_) => {
                let this: UInt = self.cast_into();
                (this - other.cast_into()).into()
            }
            NT::Number => panic!("A number instance must have a specific type, not Number"),
        }
    }
}

impl Default for Number {
    fn default() -> Number {
        Number::Bool(Boolean::default())
    }
}

impl From<bool> for Number {
    fn from(b: bool) -> Number {
        Number::Bool(b.into())
    }
}

impl From<u64> for Number {
    fn from(u: u64) -> Number {
        Number::UInt(u.into())
    }
}

impl From<Boolean> for Number {
    fn from(b: Boolean) -> Number {
        Number::Bool(b)
    }
}

impl From<Complex> for Number {
    fn from(c: Complex) -> Number {
        Number::Complex(c)
    }
}

impl From<Float> for Number {
    fn from(f: Float) -> Number {
        Number::Float(f)
    }
}

impl From<Int> for Number {
    fn from(i: Int) -> Number {
        Number::Int(i)
    }
}

impl From<UInt> for Number {
    fn from(u: UInt) -> Number {
        Number::UInt(u)
    }
}

impl CastFrom<Number> for Boolean {
    fn cast_from(number: Number) -> Boolean {
        if number == number.class().zero() {
            Boolean(false)
        } else {
            Boolean(true)
        }
    }
}

impl CastFrom<Number> for Float {
    fn cast_from(number: Number) -> Float {
        use Number::*;
        match number {
            Bool(b) => Self::cast_from(b),
            Complex(c) => Self::cast_from(c),
            Float(f) => f,
            Int(i) => Self::cast_from(i),
            UInt(u) => Self::cast_from(u),
        }
    }
}

impl CastFrom<Number> for Int {
    fn cast_from(number: Number) -> Int {
        use Number::*;
        match number {
            Bool(b) => Self::cast_from(b),
            Complex(c) => Self::cast_from(c),
            Float(f) => Self::cast_from(f),
            Int(i) => i,
            UInt(u) => Self::cast_from(u),
        }
    }
}

impl CastFrom<Number> for UInt {
    fn cast_from(number: Number) -> UInt {
        use Number::*;
        match number {
            Bool(b) => Self::cast_from(b),
            Complex(c) => Self::cast_from(c),
            Float(f) => Self::cast_from(f),
            Int(i) => Self::cast_from(i),
            UInt(u) => u,
        }
    }
}

impl TryFrom<Number> for bool {
    type Error = error::TCError;

    fn try_from(n: Number) -> TCResult<bool> {
        match n {
            Number::Bool(b) => Ok(b.into()),
            other => Err(error::bad_request("Expected Boolean but found", other)),
        }
    }
}

impl TryFrom<Number> for Boolean {
    type Error = error::TCError;

    fn try_from(n: Number) -> TCResult<Boolean> {
        match n {
            Number::Bool(b) => Ok(b),
            other => Err(error::bad_request("Expected Boolean but found", other)),
        }
    }
}

impl TryFrom<Number> for Complex {
    type Error = error::TCError;

    fn try_from(n: Number) -> TCResult<Complex> {
        match n {
            Number::Complex(c) => Ok(c),
            other => Err(error::bad_request("Expected Complex but found", other)),
        }
    }
}

impl TryFrom<Number> for Float {
    type Error = error::TCError;

    fn try_from(n: Number) -> TCResult<Float> {
        match n {
            Number::Float(f) => Ok(f),
            other => Err(error::bad_request("Expected Float but found", other)),
        }
    }
}

impl TryFrom<Number> for Int {
    type Error = error::TCError;

    fn try_from(n: Number) -> TCResult<Int> {
        match n {
            Number::Int(i) => Ok(i),
            other => Err(error::bad_request("Expected Int but found", other)),
        }
    }
}

impl TryFrom<Number> for UInt {
    type Error = error::TCError;

    fn try_from(n: Number) -> TCResult<UInt> {
        match n {
            Number::UInt(u) => Ok(u),
            other => Err(error::bad_request("Expected UInt but found", other)),
        }
    }
}

impl TryFrom<Number> for u64 {
    type Error = error::TCError;

    fn try_from(n: Number) -> TCResult<u64> {
        let u: UInt = n.try_into()?;
        Ok(u.into())
    }
}

impl Serialize for Number {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Number::Bool(b) => b.serialize(s),
            Number::Complex(c) => c.serialize(s),
            Number::Float(f) => f.serialize(s),
            Number::Int(i) => i.serialize(s),
            Number::UInt(u) => u.serialize(s),
        }
    }
}

impl fmt::Debug for Number {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Number::Bool(b) => write!(f, "Bool({})", b),
            Number::Complex(c) => write!(f, "Complex({})", c),
            Number::Float(n) => write!(f, "Float({})", n),
            Number::Int(i) => write!(f, "Int({})", i),
            Number::UInt(u) => write!(f, "UInt({})", u),
        }
    }
}
