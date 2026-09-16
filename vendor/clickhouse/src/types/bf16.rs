use half::bf16;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Ordering;
use std::fmt::{Debug, Display, Formatter};

/// A 16-bit brain floating point number (bfloat16).
///
/// # Note: Not for General Use
/// This type is primarily intended for input/output purposes,
/// and is not designed to be a general-purpose bfloat16 type.
///
/// Notably, this does not support:
/// * any of the arithmetic operators/methods
/// * formatting in different radixes
/// * parsing from strings
///
/// It should be possible to convert to other crates' types using [`Self::to_bits()`]
/// and using the corresponding `from_bits()` method on the other type,
/// and vice versa with [`Self::from_bits()`].
///
/// Conversions to/from standard float types are supported:
/// * `f32`, `f64` via `From` and `Into`

#[derive(Copy, Clone, PartialEq)]
pub struct BFloat16 {
    bits: u16,
}

pub(crate) const MODULE_PATH: &str = module_path!();
pub(crate) const BYTE_LEN: usize = 2;

impl BFloat16 {
    /// Not a Number.
    pub const NAN: Self = Self { bits: 0x7FC0 };

    /// Positive infinity.
    pub const INFINITY: Self = Self { bits: 0x7F80 };

    /// Negative infinity.
    pub const NEG_INFINITY: Self = Self { bits: 0xFF80 };

    /// Smallest positive value.
    pub const MIN_POSITIVE: Self = Self { bits: 0x0080 };

    /// Largest finite value.
    pub const MAX: Self = Self { bits: 0x7F7F };

    /// Smallest (most negative) finite value.
    pub const MIN: Self = Self { bits: 0xFF7F };

    /// Zero.
    pub const ZERO: Self = Self { bits: 0x0000 };

    /// Negative zero.
    pub const NEG_ZERO: Self = Self { bits: 0x8000 };

    /// Value `1.0`.
    pub const ONE: Self = Self { bits: 0x3F80 };

    pub(crate) const SERDE_NAME: &'static str = concat!(module_path!(), "::BFloat16");

    pub const fn from_bits(bits: u16) -> Self {
        Self { bits }
    }

    pub const fn to_bits(self) -> u16 {
        self.bits
    }

    pub fn from_f32(val: f32) -> Self {
        Self {
            bits: bf16::from_f32(val).to_bits(),
        }
    }

    pub fn to_f32(self) -> f32 {
        bf16::from_bits(self.bits).to_f32()
    }

    pub fn from_f64(val: f64) -> Self {
        Self {
            bits: bf16::from_f64(val).to_bits(),
        }
    }

    pub fn to_f64(self) -> f64 {
        bf16::from_bits(self.bits).to_f64()
    }

    pub fn is_nan(self) -> bool {
        bf16::from_bits(self.bits).is_nan()
    }

    pub fn is_infinite(self) -> bool {
        bf16::from_bits(self.bits).is_infinite()
    }

    pub fn is_finite(self) -> bool {
        bf16::from_bits(self.bits).is_finite()
    }
}

impl PartialOrd for BFloat16 {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        bf16::from_bits(self.bits).partial_cmp(&bf16::from_bits(other.bits))
    }
}

impl Display for BFloat16 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.to_f32(), f)
    }
}

impl Debug for BFloat16 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("BFloat16").field(&self.to_f32()).finish()
    }
}

// From f32/f64 -> BFloat16
impl From<f32> for BFloat16 {
    fn from(val: f32) -> Self {
        Self::from_f32(val)
    }
}

impl From<f64> for BFloat16 {
    fn from(val: f64) -> Self {
        Self::from_f64(val)
    }
}

// BFloat16 -> f32/f64
impl From<BFloat16> for f32 {
    fn from(val: BFloat16) -> Self {
        val.to_f32()
    }
}

impl From<BFloat16> for f64 {
    fn from(val: BFloat16) -> Self {
        val.to_f64()
    }
}

impl Serialize for BFloat16 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer
            .serialize_newtype_struct(Self::SERDE_NAME, &SerializeBytes2(&self.bits.to_le_bytes()))
    }
}

impl<'de> Deserialize<'de> for BFloat16 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = deserializer.deserialize_newtype_struct(Self::SERDE_NAME, VisitBytes2)?;
        Ok(Self {
            bits: u16::from_le_bytes(bytes),
        })
    }
}

struct VisitBytes2;

impl<'de> serde::de::Visitor<'de> for VisitBytes2 {
    type Value = [u8; 2];

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad("expected 2-byte array")
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
        v.try_into()
            .map_err(|_| serde::de::Error::custom(format!("expected 2 bytes, got {}", v.len())))
    }
}

struct SerializeBytes2<'a>(&'a [u8; 2]);

impl Serialize for SerializeBytes2<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::BFloat16;

    #[test]
    fn bfloat16_consts() {
        assert_eq!(BFloat16::ZERO.to_f32(), 0.0f32);
        assert_eq!(BFloat16::ONE.to_f32(), 1.0f32);
        assert!(BFloat16::NAN.is_nan());
        assert!(BFloat16::INFINITY.is_infinite());
        assert!(BFloat16::NEG_INFINITY.is_infinite());
        assert!(BFloat16::MAX.to_f32() > 0.0);
        assert!(BFloat16::MIN.to_f32() < 0.0);

        assert!(
            BFloat16::SERDE_NAME.starts_with(super::MODULE_PATH),
            "expected `BFloat16::SERDE_NAME` ({:?}) to start with `super::MODULE_PATH` ({:?})",
            BFloat16::SERDE_NAME,
            super::MODULE_PATH,
        );
    }

    #[test]
    fn bfloat16_conversions() {
        let values = [0.0f32, 1.0, -1.0, 0.5, 100.0, -100.0];

        for value in values {
            let bf = BFloat16::from_f32(value);
            let roundtrip = bf.to_f32();
            assert!(
                (roundtrip - value).abs() <= value.abs() * 0.01 + 1e-6,
                "round-trip failed for {value}: got {roundtrip}"
            );
            assert_eq!(format!("{bf:?}"), format!("BFloat16({roundtrip:?})"));
        }
    }
}
