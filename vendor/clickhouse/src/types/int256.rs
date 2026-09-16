use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Ordering;
use std::fmt::{Debug, Display, Formatter};

pub(crate) const MODULE_PATH: &str = module_path!();
pub(crate) const BYTE_LEN: usize = 32;

/// A 256-bit unsigned integer.
///
/// See [the `UInt256` type in the ClickHouse reference](https://clickhouse.com/docs/sql-reference/data-types/int-uint)
/// for details.
///
/// # Note: Not for General Use
/// This type (and [`Int256`]) are primarily intended for input/output with ClickHouse,
/// and are not designed to be general-purpose 256-bit integer types.
///
/// Notably, these do not support:
/// * any of the arithmetic operators/methods
/// * formatting in different radixes
/// * parsing from strings
///
/// Additionally, the Serde trait impls are not guaranteed to work with
/// anything but the serializers and deserializers defined in the `clickhouse` crate.
///
/// It should be possible to convert to other crates' types using [`Self::to_le_bytes()`]
/// and using the corresponding `from_le_bytes()` method (or similar) on the other type,
/// and vice versa with [`Self::from_le_bytes()`].
///
/// Conversions to/from the standard integer types are also supported:
///
/// * `u8`, `u16`, `u32`, `u64` via `TryInto` and `From`
/// * `i8`, `i16`, `i32`, `i64` via `TryFrom` and `TryInto`
///
/// Support for external crates as optional features may be implemented on request.
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct UInt256 {
    le_bytes: [u8; 32],
}

impl PartialOrd for UInt256 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for UInt256 {
    fn cmp(&self, other: &Self) -> Ordering {
        self.le_bytes.iter().rev().cmp(other.le_bytes.iter().rev())
    }
}

impl UInt256 {
    /// Minimum value for this type (`0`).
    pub const MIN: Self = Self::ZERO;

    /// Value `0` for this type.
    pub const ZERO: Self = Self { le_bytes: [0; 32] };

    /// Value `1` for this type.
    pub const ONE: Self = Self {
        // `0b0000..0001`
        le_bytes: const {
            let mut le_bytes = [0u8; 32];
            le_bytes[0] = 1;
            le_bytes
        },
    };

    /// Maximum value for this type (`2^256 - 1`).
    pub const MAX: Self = Self {
        le_bytes: [0xFF; 32],
    };

    pub(crate) const SERDE_NAME: &'static str = concat!(module_path!(), "::UInt256");

    #[inline]
    fn widen_from_le_bytes(bytes: &[u8]) -> Self {
        Self::from_le_bytes(widen_from_le_bytes(bytes))
    }

    #[inline]
    pub const fn from_le_bytes(bytes: [u8; 32]) -> Self {
        Self { le_bytes: bytes }
    }

    pub const fn to_le_bytes(self) -> [u8; 32] {
        self.le_bytes
    }

    #[inline]
    fn narrow_to_le_bytes<const N: usize>(self) -> Option<[u8; N]> {
        let (le_bytes, remainder) = self.le_bytes.split_first_chunk()?;

        if remainder.iter().all(|byte| *byte == 0) {
            Some(*le_bytes)
        } else {
            None
        }
    }

    // `bnum` is a pretty solid zero-deps crate, but would be a SemVer-hazard to expose
    // at this point because they have some pretty big breaking changes planned:
    // https://github.com/isaacholt100/bnum/discussions/54
    #[inline]
    fn as_bnum(&self) -> bnum::types::U256 {
        // `U256::from_le_bytes()` requires nightly:
        // https://docs.rs/bnum/latest/bnum/struct.BUint.html#method.from_le_slice
        //
        // We don't just directly wrap `U256` because `to_le_bytes()` *also* requires nightly
        bnum::types::U256::from_le_slice(&self.le_bytes)
            .expect("BUG: conversion to `U256` should not fail")
    }
}

// A manual impl showing the actual integer value is probably more useful than
// a derived impl showing the bytes.
impl Debug for UInt256 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        // Make it clear this is a wrapper.
        f.debug_tuple("UInt256").field(&self.as_bnum()).finish()
    }
}

impl Display for UInt256 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.as_bnum(), f)
    }
}

impl Serialize for UInt256 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            // Tell the serializer who we are so we can get special treatment.
            // This might seem like a crazy hack, but it's exactly what
            // `serde_json::value::RawValue` does:
            // https://docs.rs/serde_json/1.0.145/src/serde_json/raw.rs.html#306
            serializer
                .serialize_newtype_struct(Self::SERDE_NAME, &format_args!("{}", self.as_bnum()))
        } else {
            serializer.serialize_newtype_struct(
                Self::SERDE_NAME,
                // Passing `[u8; 32]` would serialize as a tuple of integers
                &SerializeBytes32(&self.le_bytes),
            )
        }
    }
}

impl<'de> Deserialize<'de> for UInt256 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self {
            le_bytes: deserializer.deserialize_newtype_struct(Self::SERDE_NAME, VisitBytes32)?,
        })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("`UInt256` value out of range for the target type")]
pub struct TryFromUInt256Error(());

macro_rules! impl_uint256_from {
    ($($ty:ty),*) => {$(
        impl From<$ty> for UInt256 {
            #[inline]
            fn from(val: $ty) -> Self {
                Self::widen_from_le_bytes(&val.to_le_bytes())
            }
        }

        impl From<&'_ $ty> for UInt256 {
            #[inline]
            fn from(val: &'_ $ty) -> Self {
                Self::widen_from_le_bytes(&val.to_le_bytes())
            }
        }
    )*}
}

impl_uint256_from! { u8, u16, u32, u64, u128 }

macro_rules! impl_try_from_uint256 {
    ($($ty:ty),*) => {$(
        impl TryFrom<UInt256> for $ty {
            type Error = TryFromUInt256Error;

            /// Convert a `UInt256` to a smaller unsigned type.
            ///
            /// Errors if the value is too large to fit into the target type.
            fn try_from(value: UInt256) -> Result<Self, Self::Error> {
                Ok(<$ty>::from_le_bytes(value.narrow_to_le_bytes().ok_or(TryFromUInt256Error(()))?))
            }
        }

        impl TryFrom<&'_ UInt256> for $ty {
            type Error = TryFromUInt256Error;

            /// Convert a `UInt256` to a smaller unsigned type.
            ///
            /// Errors if the value is too large to fit into the target type.
            fn try_from(value: &'_ UInt256) -> Result<Self, Self::Error> {
                Ok(<$ty>::from_le_bytes(value.narrow_to_le_bytes().ok_or(TryFromUInt256Error(()))?))
            }
        }
    )*};
}

impl_try_from_uint256! { u8, u16, u32, u64, u128 }

macro_rules! impl_try_from_uint256_cast_signed {
    ($($unsigned:ty => $signed:ty),*) => {$(
        impl TryFrom<UInt256> for $signed {
            type Error = TryFromUInt256Error;

            /// Convert a `UInt256` to a smaller, signed type.
            ///
            /// Errors if the value is too large to fit into the target type,
            /// or if the conversion would wrap into the negative.
            fn try_from(value: UInt256) -> Result<Self, Self::Error> {
                <$unsigned>::try_from(value)?
                    .try_into()
                    .map_err(|_| TryFromUInt256Error(()))
            }
        }

        impl TryFrom<&'_ UInt256> for $signed {
            type Error = TryFromUInt256Error;

            /// Convert a `UInt256` to a smaller, signed type.
            ///
            /// Errors if the value is too large to fit into the target type,
            /// or if the conversion would wrap into the negative.
            fn try_from(value: &'_ UInt256) -> Result<Self, Self::Error> {
                <$unsigned>::try_from(value)?
                    .try_into()
                    .map_err(|_| TryFromUInt256Error(()))
            }
        }
    )*};
}

impl_try_from_uint256_cast_signed! { u8 => i8, u16 => i16, u32 => i32, u64 => i64, u128 => i128 }

/// A 256-bit signed integer in two's complement.
///
/// See [the `UInt256` type in the ClickHouse reference](https://clickhouse.com/docs/sql-reference/data-types/int-uint)
/// for details.
///
/// # Note: Not for General Use
/// This type (and [`UInt256`]) are primarily intended for input/output with ClickHouse,
/// and are not designed to be general-purpose 256-bit integer types.
///
/// Notably, these do not support:
/// * any of the arithmetic operators/methods
/// * formatting in different radixes
/// * parsing from strings
///
/// Additionally, the Serde trait impls are not guaranteed to work with
/// anything but the serializers and deserializers defined in the `clickhouse` crate.
///
/// It should be possible to convert to other crates' types using [`Self::to_le_bytes()`]
/// and using the corresponding `from_le_bytes()` method (or similar) on the other type,
/// and vice versa with [`Self::from_le_bytes()`].
///
/// Conversions to/from the standard integer types are also supported:
///
/// * `u8`, `u16`, `u32`, `u64` via `TryInto` and `From`
/// * `i8`, `i16`, `i32`, `i64` via `TryInto` and `From`
///
/// Support for external crates as optional features may be implemented on request.
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct Int256 {
    le_bytes: [u8; 32],
}

const SIGN_BIT: u8 = 0b1000_0000;

impl Int256 {
    /// Minimum value for this type (`-2^255`).
    pub const MIN: Self = Self {
        le_bytes: const {
            // In two's complement, the lowest value is `0b1000..0000`
            let mut le_bytes = [0u8; 32];
            le_bytes[31] = SIGN_BIT;
            le_bytes
        },
    };

    /// Value `-1` for this type.
    pub const MINUS_ONE: Self = Self {
        // `0b1111..1111`
        le_bytes: [0xFF; 32],
    };

    /// Value `0` for this type.
    pub const ZERO: Self = Self {
        le_bytes: [0u8; 32],
    };

    /// Value `1` for this type.
    pub const ONE: Self = Self {
        // `0b0000..0001`
        le_bytes: const {
            let mut le_bytes = [0u8; 32];
            le_bytes[0] = 1;
            le_bytes
        },
    };

    /// Maximum value for this type (`2^255 - 1`).
    pub const MAX: Self = Self {
        le_bytes: const {
            // `0b0111..1111`
            let mut le_bytes = [0xFF; 32];
            le_bytes[31] = 0x7F;
            le_bytes
        },
    };

    pub(crate) const SERDE_NAME: &'static str = concat!(module_path!(), "::Int256");

    #[inline]
    fn widen_from_le_bytes(bytes: &[u8]) -> Self {
        Self {
            le_bytes: widen_from_le_bytes(bytes),
        }
    }

    #[inline]
    fn sign_extend_from_le_bytes(bytes: &[u8]) -> Self {
        let msb = bytes.last().expect("BUG: `bytes` cannot be empty");

        let fill = if msb & SIGN_BIT != 0 { 0xFFu8 } else { 0 };

        let mut le_bytes = [fill; 32];

        le_bytes
            .get_mut(..bytes.len())
            .unwrap_or_else(|| panic!("BUG: widening from larger slice of length {}", bytes.len()))
            .copy_from_slice(bytes);

        Self { le_bytes }
    }

    #[inline]
    pub fn from_le_bytes(bytes: [u8; 32]) -> Self {
        Self { le_bytes: bytes }
    }

    #[inline]
    pub fn to_le_bytes(self) -> [u8; 32] {
        self.le_bytes
    }

    #[inline]
    pub fn is_negative(&self) -> bool {
        is_msb_set(&self.le_bytes)
    }

    #[inline]
    fn narrow_to_le_bytes<const N: usize, const SIGNED: bool>(self) -> Option<[u8; N]> {
        let (le_bytes, remainder) = self.le_bytes.split_first_chunk()?;

        let mut all_zeroes = true;
        let mut all_ones = true;

        for &b in remainder {
            all_zeroes &= b == 0;
            all_ones &= b == 0xFF;
        }

        if all_zeroes {
            return Some(*le_bytes);
        }

        // Value is two's complement and was sign-extended
        // The optimizer should be able to statically eliminate this branch
        // and `all_ones` when `SIGNED == false`.
        if SIGNED && all_ones {
            return Some(*le_bytes);
        }

        None
    }

    // See comments on `UInt256::to_bnum()` above
    #[inline]
    fn as_bnum(&self) -> bnum::types::I256 {
        bnum::types::I256::from_le_slice(&self.le_bytes)
            .expect("BUG: conversion to `I256` should not fail")
    }
}

impl PartialOrd for Int256 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Int256 {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.is_negative(), other.is_negative()) {
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            _ => (),
        }

        // Comparing the two's complement is the same
        cmp_le(&self.le_bytes, &other.le_bytes)
    }
}

// See comments on `impl Debug for UInt256`
impl Debug for Int256 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Int256").field(&self.as_bnum()).finish()
    }
}

impl Display for Int256 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.as_bnum(), f)
    }
}

impl Serialize for Int256 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            // See comment in `UInt256::serialize()`
            serializer
                .serialize_newtype_struct(Self::SERDE_NAME, &format_args!("{}", self.as_bnum()))
        } else {
            serializer.serialize_newtype_struct(Self::SERDE_NAME, &SerializeBytes32(&self.le_bytes))
        }
    }
}

impl<'de> Deserialize<'de> for Int256 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self {
            le_bytes: deserializer.deserialize_newtype_struct(Self::SERDE_NAME, VisitBytes32)?,
        })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("`Int256` value out of range for target type")]
pub struct TryFromInt256Error(());

macro_rules! impl_int256_from_signed {
    ($($ty:ty),*) => {
        $(
            impl From<$ty> for Int256 {
                /// Widen from a smaller signed type by sign-extending.
                fn from(val: $ty) -> Self {
                    Self::sign_extend_from_le_bytes(&val.to_le_bytes())
                }
            }
        )*
    };
}

impl_int256_from_signed! {
    i8, i16, i32, i64, i128
}

macro_rules! impl_int256_from_unsigned {
    ($($ty:ty),*) => {
        $(impl From<$ty> for Int256 {
            /// Widen from a smaller unsigned type.
            fn from(val: $ty) -> Self {
                Self::widen_from_le_bytes(&val.to_le_bytes())
            }
        })*
    };
}

impl_int256_from_unsigned! {
    u8, u16, u32, u64, u128
}

macro_rules! impl_try_from_int256_signed {
    ($($ty:ty),*) => {$(
        impl TryFrom<Int256> for $ty {
            type Error = TryFromInt256Error;

            fn try_from(value: Int256) -> Result<Self, Self::Error> {
                Ok(<$ty>::from_le_bytes(value.narrow_to_le_bytes::<_, true>().ok_or(TryFromInt256Error(()))?))
            }
        }
    )*};
}

impl_try_from_int256_signed! {
    i8, i16, i32, i64, i128
}

struct VisitBytes32;

impl<'de> serde::de::Visitor<'de> for VisitBytes32 {
    type Value = [u8; 32];

    fn expecting(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.pad("expected 32-byte array")
    }

    fn visit_newtype_struct<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_bytes(self)
    }

    fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if let Ok(&bytes) = v.try_into() {
            Ok(bytes)
        } else {
            Err(serde::de::Error::custom(format!(
                "expected 32 bytes, got {}",
                v.len()
            )))
        }
    }
}

// There's `serde_bytes::ByteArray` but it doesn't save us from having to implement
// our own `Visitor` for deserialization anyway, so it doesn't really carry its weight here.
struct SerializeBytes32<'a>(&'a [u8; 32]);

impl Serialize for SerializeBytes32<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(self.0)
    }
}

#[inline]
fn widen_from_le_bytes(bytes: &[u8]) -> [u8; 32] {
    let mut le_bytes = [0u8; 32];

    le_bytes
        .get_mut(..bytes.len())
        .unwrap_or_else(|| panic!("BUG: widening from larger slice of length {}", bytes.len()))
        .copy_from_slice(bytes);

    le_bytes
}

#[inline]
fn is_msb_set(le_bytes: &[u8; 32]) -> bool {
    le_bytes[31] & SIGN_BIT != 0
}

#[inline]
fn cmp_le(left: &[u8; 32], right: &[u8; 32]) -> Ordering {
    left.iter().rev().cmp(right.iter().rev())
}

#[cfg(test)]
mod tests {
    use super::{Int256, UInt256};

    #[test]
    fn uint_256_consts() {
        assert_eq!(UInt256::MIN, UInt256::from(0u32));
        assert_eq!(UInt256::ZERO, UInt256::from(0u32));
        assert_eq!(UInt256::MAX, UInt256::from_le_bytes([0xFF; 32]));

        assert!(UInt256::MIN < UInt256::MAX);
        assert!(UInt256::ZERO < UInt256::MAX);

        assert!(UInt256::MAX > UInt256::ZERO);
        assert!(UInt256::MAX > UInt256::MIN);

        assert_eq!(UInt256::MIN, UInt256::ZERO);

        assert_eq!(UInt256::MIN.to_string(), "0");
        assert_eq!(UInt256::ZERO.to_string(), "0");
        assert_eq!(UInt256::ONE.to_string(), "1");
        // Put `2^256 - 1` into WolframAlpha to verify
        assert_eq!(
            UInt256::MAX.to_string(),
            "115792089237316195423570985008687907853269984665640564039457584007913129639935"
        );

        assert!(
            UInt256::SERDE_NAME.starts_with(super::MODULE_PATH),
            "expected `UInt256::SERDE_NAME` ({:?}) to start with `super::MODULE_PATH` ({:?})",
            UInt256::SERDE_NAME,
            super::MODULE_PATH,
        )
    }

    #[test]
    fn int_256_consts() {
        assert_eq!(Int256::ZERO, Int256::from(0u32));

        assert!(Int256::MIN < Int256::MINUS_ONE);
        assert!(Int256::MINUS_ONE < Int256::ZERO);
        assert!(Int256::ZERO < Int256::ONE);
        assert!(Int256::ONE < Int256::MAX);

        assert!(Int256::MAX > Int256::ONE);
        assert!(Int256::ONE > Int256::ZERO);
        assert!(Int256::ZERO > Int256::MINUS_ONE);
        assert!(Int256::MINUS_ONE > Int256::MIN);

        // Put `-(2^255)` into WolframAlpha to verify
        assert_eq!(
            Int256::MIN.to_string(),
            "-57896044618658097711785492504343953926634992332820282019728792003956564819968"
        );
        assert_eq!(Int256::MINUS_ONE.to_string(), "-1");
        assert_eq!(Int256::ZERO.to_string(), "0");
        assert_eq!(Int256::ONE.to_string(), "1");
        // Put `2^255 - 1` into WolframAlpha to verify
        assert_eq!(
            Int256::MAX.to_string(),
            "57896044618658097711785492504343953926634992332820282019728792003956564819967"
        );

        assert!(
            Int256::SERDE_NAME.starts_with(super::MODULE_PATH),
            "expected `Int256::SERDE_NAME` ({:?}) to start with `super::MODULE_PATH` ({:?})",
            Int256::SERDE_NAME,
            super::MODULE_PATH,
        )
    }

    #[test]
    fn uint_256_conversions() {
        let values = [0u64, 1, 1234, 1234567890, 11223344556677889900, u64::MAX];

        for value in values {
            let uint256 = UInt256::from(value);

            assert_eq!(uint256.to_string(), value.to_string());
            assert_eq!(format!("{uint256:?}"), format!("UInt256({value:?})"))
        }
    }

    #[test]
    fn int_256_conversions() {
        let values = [
            i64::MIN,
            -112233445566778899,
            -1234567890,
            -1234,
            -1,
            0,
            1,
            1234,
            1234567890,
            112233445566778899,
            i64::MAX,
        ];

        for value in values {
            let int256 = Int256::from(value);

            assert_eq!(int256.to_string(), value.to_string());
            assert_eq!(format!("{int256:?}"), format!("Int256({value:?})"))
        }
    }
}
