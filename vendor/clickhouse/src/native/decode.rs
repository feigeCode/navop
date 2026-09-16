//! Types and impls for decoding Native format.

use crate::error::BoxedError;
use crate::native::array::{ArrayData, TupleIter};
use clickhouse_types::DataTypeNode;
use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasher, Hash};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// A cursor to the data for an individual element in a [`Column`][super::Column].
pub struct ValueReader<'a> {
    pub(super) data_type: &'a DataTypeNode,
    pub(super) native_bytes: &'a [u8],
}

impl<'a> ValueReader<'a> {
    /// The data type of this value.
    pub fn data_type(&self) -> &DataTypeNode {
        self.data_type
    }

    /// The byte slice underlying this cursor.
    ///
    /// Some bytes may have been consumed from the view if [`Self::read_bytes_fixed()`]
    /// was previously called.
    pub fn native_bytes(&self) -> &'a [u8] {
        self.native_bytes
    }

    /// Read a fixed number of bytes from this cursor.
    ///
    /// # Errors
    /// If less than `LEN` bytes remain.
    pub fn read_bytes_fixed<const LEN: usize>(&mut self) -> Result<&'a [u8; LEN], ValueReadError> {
        let (ret, rem) =
            self.native_bytes
                .split_first_chunk()
                .ok_or(ValueReadError::InvalidLength {
                    expected: LEN,
                    actual: self.native_bytes.len(),
                })?;

        self.native_bytes = rem;

        Ok(ret)
    }
}

/// Errors returned by [`ValueReader`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ValueReadError {
    /// Returned by [`ValueReader::read_bytes_fixed()`].
    #[error("expected {expected} bytes, got {actual}")]
    InvalidLength {
        /// The expected number of bytes.
        expected: usize,
        /// The actual number of bytes remaining in the [`ValueReader`].
        actual: usize,
    },
}

/// Decode a Rust type from its equivalent in ClickHouse's [Native format].
///
/// [Native format]: https://clickhouse.com/docs/reference/formats/Native
pub trait Decode<'a>: 'a + Sized {
    /// Return `true` if the given data type is compatible with this Rust type.
    ///
    /// The implementation should accept both `LowCardinality(_)`
    /// and `SimpleAggregateFunction(_, _)` as valid; the data provided during decode will be
    /// identical to the unwrapped type.
    ///
    /// Use [`DataTypeNode::remove_compatible_wrappers()`] when matching on types.
    fn compatible(data_type: &DataTypeNode) -> bool;

    /// Decode the given value, which may involve parsing.
    ///
    /// The exact error type should not be considered stable.
    fn decode(reader: &mut ValueReader<'a>) -> Result<Self, BoxedError>;

    /// Decode a `NULL` value.
    ///
    /// Default implementation returns an error.
    fn decode_null(data_type: &DataTypeNode) -> Result<Self, BoxedError> {
        Err(format!("data type {data_type:?} cannot be NULL").into())
    }

    /// Decode an array of values.
    ///
    /// Default implementation returns an error.
    ///
    /// Do not override unless implementing a custom array/slice type like `Vec<T>`.
    ///
    /// Generic implementations for `Vec<T>` and `Box<[T]>` are already provided.
    fn decode_array(data: ArrayData<'a>) -> Result<Self, BoxedError> {
        Err(format!("unexpected data type Array({})", data.elem_type).into())
    }

    /// Decode a tuple of values.
    ///
    /// Default implementation returns an error.
    ///
    /// Do not override unless implementing a custom tuple struct type.
    ///
    /// Generic implementations for `(T...)` up to 16 elements are already provided.
    fn decode_tuple(data: TupleIter<'a>) -> Result<Self, BoxedError> {
        Err(format!("unexpected data type Tuple({:?})", data.types.as_slice()).into())
    }

    /// Decode a `Map`.
    ///
    /// Default implementation returns an error.
    ///
    /// Do not override unless implementing a custom map type like `HashMap<K, V>`.
    ///
    /// Generic implementations for [`HashMap`] and [`BTreeMap`] are already provided.
    fn decode_map(key_data: ArrayData<'a>, value_data: ArrayData<'a>) -> Result<Self, BoxedError> {
        Err(format!(
            "unexpected data type Map({}, {})",
            key_data.elem_type, value_data.elem_type
        )
        .into())
    }
}

macro_rules! type_matches {
    ($data_type:expr, $data_type_pat:pat) => {
        match &$data_type {
            $data_type_pat => true,
            $crate::native::DataTypeNode::LowCardinality(inner) => {
                matches!(**inner, $data_type_pat)
            }
            $crate::native::DataTypeNode::SimpleAggregateFunction(_, inner) => {
                matches!(**inner, $data_type_pat)
            }
            _ => false,
        }
    };
}

macro_rules! impl_from_le_bytes {
    ($($dataty:ident: $ty:ident),* $(,)?) => {
        $(
            impl<'a> Decode<'a> for $ty {
                fn compatible(data_type: &DataTypeNode) -> bool {
                    type_matches!(data_type, DataTypeNode::$dataty)
                }

                #[doc=concat!("Decode this type from `", stringify!($dataty), "`.")]
                fn decode(
                    reader: &mut ValueReader<'a>,
                ) -> Result<Self, BoxedError> {
                    Ok($ty::from_le_bytes(*reader.read_bytes_fixed()?))
                }
            }
        )*
    };
}

// All scalar primitives are in little-endian
impl_from_le_bytes!(
    // 8-bit ints don't have a concept of "endianness" but they still implement `from_bytes_le()`
    // for the express purpose of being included in macros like this
    Int8: i8,
    Int16: i16,
    Int32: i32,
    Int64: i64,
    Int128: i128,
    UInt8: u8,
    UInt16: u16,
    UInt32: u32,
    UInt64: u64,
    UInt128: u128,
    Float32: f32,
    Float64: f64,
);

impl Decode<'_> for bool {
    fn compatible(data_type: &DataTypeNode) -> bool {
        type_matches!(data_type, DataTypeNode::Bool)
    }

    /// Decode this type from `Bool`.
    fn decode(reader: &mut ValueReader<'_>) -> Result<Self, BoxedError> {
        // https://clickhouse.com/docs/interfaces/specs/NativeFormat#bool
        let [b] = reader.read_bytes_fixed()?;
        Ok(*b != 0)
    }
}

// Note: no impl for `&[u8]` because it could be confused with the blanket impl for `Vec<u8>`
/// Zero-copy decoding for strings.
impl<'a> Decode<'a> for &'a str {
    fn compatible(data_type: &DataTypeNode) -> bool {
        type_matches!(
            data_type,
            DataTypeNode::String | DataTypeNode::FixedString(_)
        )
    }

    /// Decode this type from `String` or `FixedString`.
    ///
    /// # Errors
    /// If the string data is not valid UTF-8.
    fn decode(reader: &mut ValueReader<'a>) -> Result<Self, BoxedError> {
        Ok(str::from_utf8(reader.native_bytes)?)
    }
}

impl<'a> Decode<'a> for String {
    fn compatible(data_type: &DataTypeNode) -> bool {
        <&str as Decode>::compatible(data_type)
    }

    /// Decode this type from `String` or `FixedString`.
    ///
    /// # Errors
    /// If the string data is not valid UTF-8.
    fn decode(reader: &mut ValueReader<'a>) -> Result<Self, BoxedError> {
        Ok(<&str as Decode>::decode(reader)?.into())
    }
}

impl<'a, T: Decode<'a>> Decode<'a> for Option<T> {
    fn compatible(data_type: &DataTypeNode) -> bool {
        if let DataTypeNode::Nullable(inner) = data_type {
            T::compatible(inner)
        } else {
            false
        }
    }

    fn decode(reader: &mut ValueReader<'a>) -> Result<Self, BoxedError> {
        let DataTypeNode::Nullable(inner_type) = reader.data_type else {
            return Err(format!("expected `Nullable(_)`, got {:?}", reader.data_type).into());
        };

        Ok(Some(T::decode(&mut ValueReader {
            data_type: inner_type,
            native_bytes: reader.native_bytes,
        })?))
    }

    fn decode_null(_data_type: &DataTypeNode) -> Result<Self, BoxedError> {
        Ok(None)
    }

    fn decode_array(data: ArrayData<'a>) -> Result<Self, BoxedError> {
        T::decode_array(data).map(Some)
    }

    fn decode_tuple(data: TupleIter<'a>) -> Result<Self, BoxedError> {
        T::decode_tuple(data).map(Some)
    }
}

impl<'a, T: Decode<'a> + 'a> Decode<'a> for Vec<T> {
    fn compatible(data_type: &DataTypeNode) -> bool {
        if let DataTypeNode::Array(elem_type) = data_type {
            T::compatible(elem_type.remove_compatible_wrappers())
        } else {
            false
        }
    }

    fn decode(reader: &mut ValueReader<'a>) -> Result<Self, BoxedError> {
        Err(format!("expected array type, got {}", reader.data_type).into())
    }

    fn decode_array(data: ArrayData<'a>) -> Result<Self, BoxedError> {
        data.into_reader::<T>()?
            .collect::<Result<Vec<T>, crate::Error>>()
            .map_err(Into::into)
    }
}

macro_rules! tuple_impl {
    ($var1:ident: $ty1:ident $(, $var:ident: $ty:ident)*) => {
        impl<'a, $ty1 $(, $ty)* > Decode<'a> for ($ty1, $($ty),*)
            where
                $ty1: Decode<'a>,
                $($ty: Decode<'a>,)*
        {
            fn compatible(data_type: &DataTypeNode) -> bool {
                let DataTypeNode::Tuple(types) = data_type else {
                    return false;
                };

                // Matches exact length of array
                let [$var1 $(, $var)*] = &types[..] else {
                    return false;
                };

                <$ty1 as Decode>::compatible($var1.remove_compatible_wrappers())
                $(&& <$ty as Decode>::compatible($var.remove_compatible_wrappers()))*

            }

            fn decode(
                reader: &mut ValueReader<'a>,
            ) -> Result<Self, BoxedError> {
                Err(format!("expected array type, got {}", reader.data_type).into())
            }

            fn decode_tuple(mut data: TupleIter<'a>) -> Result<Self, BoxedError> {
                Ok((
                    data.decode_next::<$ty1>()?,
                    $(data.decode_next::<$ty>()?),*
                ))
            }
        }

        tuple_impl!($($var: $ty),*);
    };
    () => {}
}

// `serde::Deserialize` is implemented for tuples up to 16 items
tuple_impl!(
    t1: T1, t2: T2, t3: T3, t4: T4, t5: T5, t6: T6, t7: T7, t8: T8, t9: T9,
    t10: T10, t11: T11, t12: T12, t13: T13, t14: T14, t15: T15, t16: T16
);

impl<'a, K, V, S> Decode<'a> for HashMap<K, V, S>
where
    K: Decode<'a> + Hash + Eq,
    V: Decode<'a>,
    S: BuildHasher + Default + 'a,
{
    fn compatible(data_type: &DataTypeNode) -> bool {
        let DataTypeNode::Map([key_ty, val_ty]) = data_type else {
            return false;
        };

        K::compatible(key_ty.remove_compatible_wrappers())
            && V::compatible(val_ty.remove_compatible_wrappers())
    }

    fn decode(reader: &mut ValueReader<'a>) -> Result<Self, BoxedError> {
        Err(format!("expected map, got {}", reader.data_type).into())
    }

    fn decode_map(key_data: ArrayData<'a>, value_data: ArrayData<'a>) -> Result<Self, BoxedError> {
        key_data
            .into_reader::<K>()?
            .zip(value_data.into_reader::<V>()?)
            .map(|(k, v)| Ok((k?, v?)))
            .collect::<Result<Self, BoxedError>>()
    }
}

impl<'a, K, V> Decode<'a> for BTreeMap<K, V>
where
    K: Decode<'a> + Ord + Eq,
    V: Decode<'a>,
{
    fn compatible(data_type: &DataTypeNode) -> bool {
        let DataTypeNode::Map([key_ty, val_ty]) = data_type else {
            return false;
        };

        K::compatible(key_ty.remove_compatible_wrappers())
            && V::compatible(val_ty.remove_compatible_wrappers())
    }

    fn decode(reader: &mut ValueReader<'a>) -> Result<Self, BoxedError> {
        Err(format!("expected map, got {}", reader.data_type).into())
    }

    fn decode_map(key_data: ArrayData<'a>, value_data: ArrayData<'a>) -> Result<Self, BoxedError> {
        key_data
            .into_reader::<K>()?
            .zip(value_data.into_reader::<V>()?)
            .map(|(k, v)| Ok((k?, v?)))
            .collect::<Result<Self, BoxedError>>()
    }
}

impl Decode<'_> for Ipv4Addr {
    fn compatible(data_type: &DataTypeNode) -> bool {
        type_matches!(data_type, DataTypeNode::IPv4)
    }

    fn decode(reader: &mut ValueReader<'_>) -> Result<Self, BoxedError> {
        // https://clickhouse.com/docs/interfaces/specs/NativeFormat#ipv4-and-ipv6
        // IPv4 is byte-reversed, so little-endian
        let bytes_le = u32::from_le_bytes(*reader.read_bytes_fixed()?);
        // Performs the byte-swap internally
        Ok(Ipv4Addr::from_bits(bytes_le))
    }
}

impl Decode<'_> for Ipv6Addr {
    fn compatible(data_type: &DataTypeNode) -> bool {
        type_matches!(data_type, DataTypeNode::IPv6)
    }

    fn decode(reader: &mut ValueReader<'_>) -> Result<Self, BoxedError> {
        // https://clickhouse.com/docs/interfaces/specs/NativeFormat#ipv4-and-ipv6
        // IPv6 uses canonical (big-endian) encoding
        Ok(Ipv6Addr::from(*reader.read_bytes_fixed::<16>()?))
    }
}

/// Accepts either a `IPv4` or `IPv6` type.
///
/// # Note: No `Encode` Impl
/// ClickHouse does not have a polymorphic IP address type, so this cannot be infallibly encoded.
impl Decode<'_> for IpAddr {
    fn compatible(data_type: &DataTypeNode) -> bool {
        type_matches!(data_type, DataTypeNode::IPv4 | DataTypeNode::IPv6)
    }

    fn decode(reader: &mut ValueReader<'_>) -> Result<Self, BoxedError> {
        match reader.data_type {
            DataTypeNode::IPv4 => Ipv4Addr::decode(reader).map(Into::into),
            DataTypeNode::IPv6 => Ipv6Addr::decode(reader).map(Into::into),
            other => Err(format!("expected IP address, got {other}").into()),
        }
    }
}

#[cfg(feature = "uuid")]
mod uuid {
    use super::{Decode, ValueReader};
    use crate::error::BoxedError;
    use clickhouse_types::DataTypeNode;
    use uuid::Uuid;

    impl Decode<'_> for Uuid {
        fn compatible(data_type: &DataTypeNode) -> bool {
            type_matches!(data_type, DataTypeNode::UUID)
        }

        fn decode(reader: &mut ValueReader<'_>) -> Result<Self, BoxedError> {
            // https://clickhouse.com/docs/interfaces/specs/NativeFormat#uuid
            // Wire bytes 0..7 = canonical bytes 0..7 reversed.
            // Wire bytes 8..15 = canonical bytes 8..15 reversed.
            let hi_bytes = u64::from_le_bytes(*reader.read_bytes_fixed()?);
            let low_bytes = u64::from_le_bytes(*reader.read_bytes_fixed()?);

            Ok(Uuid::from_u64_pair(hi_bytes, low_bytes))
        }
    }
}
