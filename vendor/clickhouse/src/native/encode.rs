//! Types and impls for encoding Native format.

use crate::error::BoxedError;
use crate::native::builder::{LayoutBuilder, LayoutBuilderKind};
use crate::native::utils::DebugNullMap;
use bytes::{BufMut, BytesMut};
use clickhouse_types::DataTypeNode;
use std::cmp;
use std::collections::{BTreeMap, HashMap};
use std::marker::PhantomData;
use std::net::{Ipv4Addr, Ipv6Addr};

/// Encode a Rust type to its equivalent in ClickHouse's [Native format].
///
/// [Native format]: https://clickhouse.com/docs/reference/formats/Native
pub trait Encode {
    /// The ClickHouse data type this type will encode to.
    fn produces() -> DataTypeNode;

    /// Encode a value.
    ///
    /// Any error may be returned. The exact error type should not be considered stable.
    fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError>;

    /// Returns `true` if a column of the given data type can accept this type.
    ///
    /// The default implementation allows a Rust type producing a ClickHouse type `T`
    /// to encode to:
    ///
    /// * `T`
    /// * `Nullable(T)`
    /// * `LowCardinality(T)`
    /// * `SimpleAggregateFunction(_, T)`
    fn compatible(column_type: &DataTypeNode) -> bool {
        let produced_type = Self::produces();

        default_compatible(&produced_type, column_type)
    }
}

fn default_compatible(produced_type: &DataTypeNode, column_type: &DataTypeNode) -> bool {
    recursive_compatible(column_type, |column_type| {
        if produced_type == column_type {
            return true;
        }

        match (produced_type, column_type) {
            // SimpleAggregateFunction has the same wire image as the underlying type
            // and the server should implicitly expand LowCardinality
            (
                DataTypeNode::LowCardinality(left) | DataTypeNode::SimpleAggregateFunction(_, left),
                right,
            ) => default_compatible(left, right),
            // Not-null value can be written to nullable column but not vice versa
            (DataTypeNode::Nullable(left), DataTypeNode::Nullable(right)) => {
                default_compatible(left, right)
            }
            (
                left,
                DataTypeNode::LowCardinality(right)
                | DataTypeNode::SimpleAggregateFunction(_, right)
                | DataTypeNode::Nullable(right),
            ) => default_compatible(left, right),
            _ => false,
        }
    })
}

fn recursive_compatible<F: Fn(&DataTypeNode) -> bool>(
    column_type: &DataTypeNode,
    compatible: F,
) -> bool {
    if compatible(column_type) {
        return true;
    }

    match column_type {
        // SimpleAggregateFunction has the same wire image as the underlying type
        // and the server should implicitly expand LowCardinality
        // Not-null value can be written to nullable column but not vice versa
        DataTypeNode::LowCardinality(inner)
        | DataTypeNode::SimpleAggregateFunction(_, inner)
        | DataTypeNode::Nullable(inner) => recursive_compatible(inner, compatible),
        _ => false,
    }
}

/// Writes an individual value to a column.
pub struct ValueWriter<'a> {
    pub(super) data_type: &'a DataTypeNode,
    pub(super) layout: &'a mut LayoutBuilder,
}

/// Error returned by methods of [`ValueWriter`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ValueWriteError {
    /// The wrong method was called for the column type, e.g. [`ValueWriter::write_array()`]
    /// was called for a column of type `UInt32`.
    #[error("attempting to use incorrect writer method for this type")]
    IncorrectMethod,

    /// Attempted to write a null value (`Option::None`) to a column that is not `Nullable(_)`.
    #[error("column does not allow nullable values here")]
    UnexpectedNull,

    /// Attempted to write a number of bytes to a fixed-width type of a different size.
    #[error("expected {expected} bytes, got {actual}")]
    InvalidLength {
        /// The size of the fixed-width type.
        expected: usize,
        /// The number of bytes that were written.
        actual: usize,
    },
}

impl<'a> ValueWriter<'a> {
    /// The data type of the target column.
    pub fn column_type(&self) -> &'a DataTypeNode {
        self.data_type
    }

    /// Write a value to a fixed-width column.
    ///
    /// # Errors
    /// * [`ValueWriteError::IncorrectMethod`] if the data type is not fixed-width.
    /// * [`ValueWriteError::InvalidLength`] if the data type is a different width
    ///   than the number of bytes written.
    pub fn write_fixed(&mut self, bytes: &[u8]) -> Result<(), ValueWriteError> {
        let LayoutBuilderKind::Fixed {
            type_width,
            ref mut data,
        } = self.layout.kind
        else {
            return Err(ValueWriteError::IncorrectMethod);
        };

        if bytes.len() != type_width {
            return Err(ValueWriteError::InvalidLength {
                expected: type_width,
                actual: bytes.len(),
            });
        }

        data.extend_from_slice(bytes);
        self.write_not_null();

        Ok(())
    }

    /// Write a string value to the column.
    ///
    /// ClickHouse strings are not required to be UTF-8, so this method accepts arbitrary bytes.
    ///
    /// # Note
    /// Types writing to a `FixedString` column should check the declared length through
    /// [`Self::column_type()`] and call [`Self::write_fixed()`] instead.
    pub fn write_string(&mut self, string_bytes: &[u8]) -> Result<(), ValueWriteError> {
        let LayoutBuilderKind::Variable { end_offsets, data } = &mut self.layout.kind else {
            return Err(ValueWriteError::IncorrectMethod);
        };

        data.extend_from_slice(string_bytes);
        end_offsets.push(data.len());

        self.write_not_null();

        Ok(())
    }

    /// Write a null value to the column.
    ///
    /// # Errors
    /// * [`ValueWriteError::UnexpectedNull`] if the column type is not `Nullable(_)`.
    pub fn write_null(&mut self) -> Result<(), ValueWriteError> {
        let nulls = self
            .layout
            .nulls
            .as_mut()
            .ok_or(ValueWriteError::UnexpectedNull)?;

        nulls.put_u8(1);

        self.layout.push_placeholder();

        Ok(())
    }

    /// Begin writing an array to the column.
    ///
    /// See [`ArrayWriter`] for details.
    ///
    /// # Errors
    /// * [`ArrayWriteError::NotAnArray`] if the column type is not `Array(_)`.
    /// * [`ArrayWriteError::IncompatibleType`] if `T` is not compatible with the column type,
    ///   as determined by [`Encode::compatible()`].
    pub fn write_array<T>(&mut self) -> Result<ArrayWriter<'_, T>, ArrayWriteError>
    where
        T: Encode,
    {
        // Note: `Nullable(Array(...))` is not allowed by ClickHouse:
        // https://clickhouse.com/docs/reference/data-types/nullable
        let DataTypeNode::Array(elem_type) = &self.data_type else {
            return Err(ArrayWriteError::NotAnArray {
                data_type: self.data_type.clone(),
            });
        };

        if !T::compatible(elem_type.remove_compatible_wrappers()) {
            return Err(ArrayWriteError::IncompatibleType {
                expected_type: (**elem_type).clone(),
            });
        }

        let LayoutBuilderKind::Array {
            end_indices,
            elem_layout,
        } = &mut self.layout.kind
        else {
            // Technically a bug if we reach this point
            unreachable!("BUG: expected LayoutBuilderKind::Array")
        };

        if let Some(nulls) = &self.layout.nulls {
            unreachable!(
                "BUG: `Nullable(Array(...))` is not allowed, but a null bitmap exists: {:?}",
                DebugNullMap(nulls)
            );
        }

        Ok(ArrayWriter {
            elem_type,
            elem_layout,
            end_indices,
            finished: false,
            _marker: PhantomData,
        })
    }

    /// Begin writing a tuple to the column.
    ///
    /// See [`TupleWriter`] for details.
    ///
    /// # Errors
    /// * [`TupleWriteError::NotATuple`] if the column type is not `Tuple(...)`
    ///   or `Nullable(Tuple(...))` (experimental type).
    pub fn write_tuple(&mut self) -> Result<TupleWriter<'_>, TupleWriteError> {
        let types = match &self.data_type {
            DataTypeNode::Tuple(types) => types,
            // Note: `Nullable(Tuple(...))` is an experimental feature
            DataTypeNode::Nullable(inner) => {
                let DataTypeNode::Tuple(types) = &**inner else {
                    return Err(TupleWriteError::NotATuple {
                        data_type: self.data_type.clone(),
                    });
                };

                types
            }
            _ => {
                return Err(TupleWriteError::NotATuple {
                    data_type: self.data_type.clone(),
                });
            }
        };

        let LayoutBuilderKind::Tuple { layouts } = &mut self.layout.kind else {
            unreachable!("BUG: expected LayoutBuilderKind::Tuple")
        };

        Ok(TupleWriter {
            index: 0,
            elem_layouts: layouts,
            elem_types: types,
            outer_nulls: self.layout.nulls.as_mut(),
            finished: false,
        })
    }

    /// Begin writing a map to the column.
    ///
    /// See [`MapWriter`] for details.
    ///
    /// # Errors
    /// * [`MapWriteError::NotAMap`] if the column type is not `Map(...)`.
    /// * [`MapWriteError::IncompatibleKeyType`] if `K` is not compatible with the map's key type,
    ///   according to [`Encode::compatible()`].
    /// * [`MapWriteError::IncompatibleValueType`] if `V` is not compatible with the map's value type,
    ///   according to [`Encode::compatible()`].
    pub fn write_map<K, V>(&mut self) -> Result<MapWriter<'_, K, V>, MapWriteError>
    where
        K: Encode,
        V: Encode,
    {
        // Note: `Nullable(Map(...))` is not allowed by ClickHouse:
        // https://clickhouse.com/docs/reference/data-types/nullable
        let DataTypeNode::Map([key_ty, val_ty]) = &self.data_type else {
            return Err(MapWriteError::NotAMap {
                data_type: self.data_type.clone(),
            });
        };

        if !K::compatible(key_ty) {
            return Err(MapWriteError::IncompatibleKeyType {
                expected_type: (**key_ty).clone(),
            });
        }

        if !V::compatible(val_ty) {
            return Err(MapWriteError::IncompatibleValueType {
                expected_type: (**val_ty).clone(),
            });
        }

        let LayoutBuilderKind::Map {
            key_val_layouts,
            end_indices,
        } = &mut self.layout.kind
        else {
            unreachable!("BUG: expected LayoutBuilderKind::Map")
        };

        if let Some(nulls) = &self.layout.nulls {
            unreachable!(
                "BUG: `Nullable(Map(...))` is not allowed, but a null bitmap exists: {:?}",
                DebugNullMap(nulls)
            );
        }

        let [key_layout, val_layout] = &mut **key_val_layouts;

        Ok(MapWriter {
            key_ty,
            val_ty,
            key_layout,
            val_layout,
            end_indices,
            finished: false,
            _marker: PhantomData,
        })
    }

    fn write_not_null(&mut self) {
        if let Some(nulls) = &mut self.layout.nulls {
            nulls.put_u8(0);
        }
    }
}

/// Writes an individual array to a column.
///
/// # Note: Rolls Back on Drop
/// [`ArrayWriter::finish()`] must be called to commit the array's contents to the column.
///
/// If the writer is dropped without calling `finish()`, the written elements are dropped and
/// no array is written to the column.
#[must_use = "rolls back the written array elements on-drop if `.finish()` is not called"]
pub struct ArrayWriter<'a, T> {
    elem_type: &'a DataTypeNode,
    elem_layout: &'a mut LayoutBuilder,
    end_indices: &'a mut Vec<usize>,
    finished: bool,
    _marker: PhantomData<fn(T)>,
}

/// Errors returned by [`ValueWriter::write_array()`] and [`ArrayWriter`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ArrayWriteError {
    /// Returned by [`ValueWriter::write_array()`] if the column is not an array.
    #[error("attempted to write an array to a non-array column: {data_type}")]
    NotAnArray {
        /// The actual column type.
        data_type: DataTypeNode,
    },

    /// Returned by [`ValueWriter::write_array()`] if the array element type is not compatible.
    #[error("value type is not compatible with expected type {expected_type}")]
    IncompatibleType {
        /// The actual column type.
        expected_type: DataTypeNode,
    },

    /// Returned by [`ArrayWriter::write()`] if [`Encode::encode()`] returns an error.
    #[error("error writing value at array index {index}")]
    ValueWriteError {
        /// The index that was being written.
        index: usize,
        /// The error from [`Encode::encode()`].
        #[source]
        error: BoxedError,
    },
}

impl<T> ArrayWriter<'_, T> {
    /// Write an element to the array.
    ///
    /// Call [`Self::finish()`] after all elements have been written.
    ///
    /// # Errors
    /// * [`ArrayWriteError::ValueWriteError`] if [`Encode::encode()`] returns an error.
    pub fn write(&mut self, value: T) -> Result<&mut Self, ArrayWriteError>
    where
        T: Encode,
    {
        value
            .encode(&mut ValueWriter {
                data_type: self.elem_type,
                layout: self.elem_layout,
            })
            .map_err(|error| ArrayWriteError::ValueWriteError {
                index: self.written_len(),
                error,
            })?;

        Ok(self)
    }

    /// Commit the array's contents to the column.
    pub fn finish(mut self) {
        self.finish_mut()
    }

    fn written_len(&self) -> usize {
        let last_array_end = self.end_indices.last().copied().unwrap_or(0);
        self.elem_layout.num_values().saturating_sub(last_array_end)
    }

    fn finish_mut(&mut self) {
        if self.finished {
            return;
        }

        self.end_indices.push(self.elem_layout.num_values());
        self.finished = true;
    }
}

impl<T> Drop for ArrayWriter<'_, T> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }

        // Roll back the written elements
        let last_array_end = self.end_indices.last().copied().unwrap_or(0);
        self.elem_layout.truncate(last_array_end);
    }
}

/// Writes an individual tuple to a column.
///
/// # Note: Rolls Back on Drop
/// [`TupleWriter::finish()`] must be called to commit the tuple's contents to the column.
///
/// If the writer is dropped without calling `finish()`, the written elements are dropped and
/// no tuple is written to the column.
#[must_use = "rolls back the written tuple elements on-drop if `.finish()` is not called"]
pub struct TupleWriter<'a> {
    index: usize,
    elem_layouts: &'a mut [LayoutBuilder],
    elem_types: &'a [DataTypeNode],
    outer_nulls: Option<&'a mut BytesMut>,
    finished: bool,
}

/// Error returned by [`ValueWriter::write_tuple()`] and [`TupleWriter`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TupleWriteError {
    /// Returned by [`ValueWriter::write_tuple()`] if the column is not a tuple.
    #[error("attempted to write a tuple to a non-tuple column: {data_type}")]
    NotATuple {
        /// The actual column type.
        data_type: DataTypeNode,
    },

    /// Returned by [`TupleWriter::write()`] when writing to a tuple that already has all its elements.
    #[error("attempted to write to a full tuple")]
    TupleFull,

    /// Returned by [`TupleWriter::write()`] when writing the wrong type to a tuple at a given position.
    #[error(
        "value type is not compatible with expected type {expected_type} at tuple index {index}"
    )]
    IncompatibleType {
        /// The 0-based position in the tuple type.
        index: usize,
        /// The expected type at that position.
        expected_type: DataTypeNode,
    },

    /// Returned by [`TupleWriter::finish()`] if the tuple had more elements to be written.
    #[error("tuple not fully written; expected {expected_len} values, got {written_len}")]
    IncompleteTuple {
        /// The expected number of elements.
        expected_len: usize,
        /// The number of elements written.
        written_len: usize,
    },

    /// Returned by [`TupleWriter::write()`] if [`Encode::encode()`] returns an error.
    #[error("error writing value at tuple index {index}")]
    ValueWriteError {
        /// The 0-based position in the tuple.
        index: usize,
        /// The error from [`Encode::encode()`].
        #[source]
        error: BoxedError,
    },
}

impl TupleWriter<'_> {
    /// Write an element to a tuple.
    ///
    /// Individual tuple elements may have different types.
    ///
    /// Call [`Self::finish()`] once all elements have been written for this tuple.
    ///
    /// # Errors
    /// * [`TupleWriteError::IncompatibleType`] if the type of the value is wrong for the current
    ///   position, as determined by [`Encode::compatible()`].
    /// * [`TupleWriteError::TupleFull`] if the tuple already has all of its elements.
    /// * [`TupleWriteError::ValueWriteError`] if [`Encode::encode()`] returns an error.
    pub fn write<T>(&mut self, value: T) -> Result<&mut Self, TupleWriteError>
    where
        T: Encode,
    {
        let data_type = self
            .elem_types
            .get(self.index)
            .ok_or(TupleWriteError::TupleFull)?;

        if !T::compatible(data_type) {
            return Err(TupleWriteError::IncompatibleType {
                index: self.index,
                expected_type: data_type.clone(),
            });
        }

        value
            .encode(&mut ValueWriter {
                layout: &mut self.elem_layouts[self.index],
                data_type,
            })
            .map_err(|error| TupleWriteError::ValueWriteError {
                index: self.index,
                error,
            })?;

        // Overflow here is likely to be a bug since
        // `self.elem_types` would have to be `usize::MAX` long
        self.index = self.index.checked_add(1).expect("tuple index overflowed");

        Ok(self)
    }

    /// Commit the tuple's contents to the column.
    ///
    /// # Errors
    /// * [`TupleWriteError::IncompleteTuple`] if more elements needed to be written.
    pub fn finish(mut self) -> Result<(), TupleWriteError> {
        if self.index < self.elem_types.len() {
            return Err(TupleWriteError::IncompleteTuple {
                expected_len: self.elem_types.len(),
                written_len: self.index,
            });
        }

        if let Some(nulls) = &mut self.outer_nulls {
            nulls.put_u8(0);
        }

        self.finished = true;

        Ok(())
    }

    fn abort_mut(&mut self) {
        if self.finished {
            return;
        }

        let written_len = cmp::min(self.index, self.elem_layouts.len());

        for layout in &mut self.elem_layouts[..written_len] {
            let len = layout.num_values();
            layout.truncate(len.saturating_sub(1));
        }
    }
}

impl Drop for TupleWriter<'_> {
    fn drop(&mut self) {
        self.abort_mut();
    }
}

/// Writes an individual `Map` to a column.
///
/// # Note: Rolls Back on Drop
/// [`MapWriter::finish()`] must be called to commit the map's contents to the column.
///
/// If the writer is dropped without calling `finish()`, the written elements are dropped and
/// no map is written to the column.
#[must_use = "rolls back the written map elements on-drop if `.finish()` is not called"]
pub struct MapWriter<'a, K, V> {
    key_ty: &'a DataTypeNode,
    val_ty: &'a DataTypeNode,
    key_layout: &'a mut LayoutBuilder,
    val_layout: &'a mut LayoutBuilder,
    end_indices: &'a mut Vec<usize>,
    finished: bool,
    _marker: PhantomData<fn(K, V)>,
}

impl<K, V> MapWriter<'_, K, V>
where
    K: Encode,
    V: Encode,
{
    /// Write an entry to the map.
    ///
    /// Call [`Self::finish()`] once all entries have been written for the `Map`.
    ///
    /// # Errors
    /// * [`MapWriteError::KeyWriteError`] if [`Encode::encode()`] returns an error for `key`.
    /// * [`MapWriteError::ValueWriteError`] if [`Encode::encode()`] returns an error for `value`.
    pub fn write(&mut self, key: K, value: V) -> Result<&mut Self, MapWriteError> {
        let index = self.key_layout.num_values();

        key.encode(&mut ValueWriter {
            layout: self.key_layout,
            data_type: self.key_ty,
        })
        .map_err(|error| MapWriteError::KeyWriteError { error, index })?;

        value
            .encode(&mut ValueWriter {
                layout: self.val_layout,
                data_type: self.val_ty,
            })
            .map_err(|error| {
                // Drop the written key so we don't get out of sync.
                self.key_layout.truncate(index);

                MapWriteError::ValueWriteError { error, index }
            })?;

        Ok(self)
    }

    /// Commit the map's contents to the column.
    pub fn finish(mut self) {
        if self.finished {
            return;
        }

        let end_index = self.key_layout.num_values();
        self.end_indices.push(end_index);

        self.finished = true;
    }
}

impl<K, V> Drop for MapWriter<'_, K, V> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }

        let truncate_len = self.end_indices.last().copied().unwrap_or(0);

        self.key_layout.truncate(truncate_len);
        self.val_layout.truncate(truncate_len);
    }
}

/// Error returned by [`ValueWriter::write_map()`] and [`MapWriter`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MapWriteError {
    /// Returned by [`ValueWriter::write_map()`] if the column type is not `Map(...)`.
    #[error("attempted to write a map to a non-map column: {data_type}")]
    NotAMap {
        /// The actual column type.
        data_type: DataTypeNode,
    },

    /// Returned by [`ValueWriter::write_map()`] if the key type is not compatible with
    /// the `Map`'s key type, as determined by [`Encode::compatible()`].
    #[error("map key type is not compatible with expected type {expected_type}")]
    IncompatibleKeyType {
        /// The actual key type.
        expected_type: DataTypeNode,
    },

    /// Returned by [`ValueWriter::write_map()`] if the key type is not compatible with
    /// the `Map`'s value type, as determined by [`Encode::compatible()`].
    #[error("map value type is not compatible with expected type {expected_type}")]
    IncompatibleValueType {
        /// The actual value type.
        expected_type: DataTypeNode,
    },

    /// Returned by [`MapWriter::write()`] if [`Encode::encode()`] returns an error for the key.
    #[error("error writing key at entry index {index}")]
    KeyWriteError {
        /// The index of the entry that was being written.
        index: usize,
        /// The error returned by [`Encode::encode()`].
        #[source]
        error: BoxedError,
    },

    /// Returned by [`MapWriter::write()`] if [`Encode::encode()`] returns an error for the value.
    #[error("error writing value at entry index {index}")]
    ValueWriteError {
        /// The index of the entry that was being written.
        index: usize,
        /// The error returned by [`Encode::encode()`].
        #[source]
        error: BoxedError,
    },
}

impl<T> Encode for Option<T>
where
    T: Encode,
{
    fn produces() -> DataTypeNode {
        DataTypeNode::Nullable(Box::new(T::produces()))
    }

    fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
        match self {
            Some(inner) => inner.encode(writer),
            None => Ok(writer.write_null()?),
        }
    }

    fn compatible(column_type: &DataTypeNode) -> bool {
        recursive_compatible(column_type, |column_type| {
            // Require the column type to be nullable
            let DataTypeNode::Nullable(inner) = column_type else {
                return false;
            };

            T::compatible(inner)
        })
    }
}

impl<T> Encode for &'_ T
where
    T: Encode + ?Sized,
{
    fn produces() -> DataTypeNode {
        T::produces()
    }

    fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
        (**self).encode(writer)
    }

    fn compatible(column_type: &DataTypeNode) -> bool {
        T::compatible(column_type)
    }
}

macro_rules! impl_to_le_bytes {
    ($($dataty:ident: $ty:ident),* $(,)?) => {
        $(
            impl Encode for $ty {
                fn produces() -> DataTypeNode {
                    DataTypeNode::$dataty
                }

                fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
                    Ok(writer.write_fixed(&self.to_le_bytes())?)
                }
            }
        )*
    };
}

// All scalar primitives are in little-endian
impl_to_le_bytes!(
    // 8-bit ints don't have a concept of "endianness" but they still implement `to_le_bytes()`
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

impl Encode for bool {
    fn produces() -> DataTypeNode {
        DataTypeNode::Bool
    }

    fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
        writer.write_fixed(&[*self as u8])?;
        Ok(())
    }
}

impl Encode for str {
    fn produces() -> DataTypeNode {
        DataTypeNode::String
    }

    fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
        if let DataTypeNode::FixedString(fixed_len) = writer.data_type {
            if self.len() != *fixed_len {
                // Give a more informative error
                return Err(format!(
                    "attempting to write a string of length {} to FixedString({fixed_len})",
                    self.len()
                )
                .into());
            }

            writer.write_fixed(self.as_bytes())?;
        } else {
            writer.write_string(self.as_bytes())?;
        }

        Ok(())
    }

    fn compatible(column_type: &DataTypeNode) -> bool {
        recursive_compatible(column_type, |column_type| {
            matches!(
                column_type,
                DataTypeNode::String | DataTypeNode::FixedString(_)
            )
        })
    }
}

impl Encode for String {
    fn produces() -> DataTypeNode {
        str::produces()
    }

    fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
        self.as_str().encode(writer)
    }

    fn compatible(column_type: &DataTypeNode) -> bool {
        str::compatible(column_type)
    }
}

impl<T> Encode for [T]
where
    T: Encode,
{
    fn produces() -> DataTypeNode {
        DataTypeNode::Array(Box::new(T::produces()))
    }

    fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
        let mut writer = writer.write_array()?;

        for val in self {
            writer.write(val)?;
        }

        writer.finish();

        Ok(())
    }

    fn compatible(column_type: &DataTypeNode) -> bool {
        recursive_compatible(column_type, |column_type| match column_type {
            DataTypeNode::Array(elem_type) => T::compatible(elem_type),
            _ => false,
        })
    }
}

impl<T> Encode for Vec<T>
where
    T: Encode,
{
    fn produces() -> DataTypeNode {
        <[T]>::produces()
    }

    fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
        self.as_slice().encode(writer)
    }

    fn compatible(column_type: &DataTypeNode) -> bool {
        <[T]>::compatible(column_type)
    }
}

macro_rules! tuple_impl {
    ($var1:ident: $ty1:ident $(, $var:ident: $ty:ident)*) => {
        impl<'a, $ty1 $(, $ty)* > Encode for ($ty1, $($ty),*)
            where
                $ty1: Encode,
                $($ty: Encode,)*
        {
            fn produces() -> DataTypeNode {
                DataTypeNode::Tuple(vec![$ty1::produces() $(, $ty::produces())*])
            }

            fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
                let mut writer = writer.write_tuple()?;

                let ($var1, $($var),*) = self;

                writer.write($var1)?;
                $(
                    writer.write($var)?;
                )*

                writer.finish()?;
                Ok(())
            }

            fn compatible(column_type: &DataTypeNode) -> bool {
                recursive_compatible(column_type, |column_type| {
                    let DataTypeNode::Tuple(types) = column_type else {
                        return false;
                    };

                    let [$var1, $($var),*] = &types[..] else {
                        return false;
                    };

                    $ty1::compatible($var1)
                        $(&& $ty::compatible($var))*
                })
            }
        }

        tuple_impl!($($var: $ty),*);
    };
    () => {}
}

tuple_impl!(
    t1: T1, t2: T2, t3: T3, t4: T4, t5: T5, t6: T6, t7: T7, t8: T8, t9: T9,
    t10: T10, t11: T11, t12: T12, t13: T13, t14: T14, t15: T15, t16: T16
);

impl Encode for Ipv4Addr {
    fn produces() -> DataTypeNode {
        DataTypeNode::IPv4
    }

    fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
        writer.write_fixed(&self.to_bits().to_le_bytes())?;
        Ok(())
    }
}

impl Encode for Ipv6Addr {
    fn produces() -> DataTypeNode {
        DataTypeNode::IPv6
    }

    fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
        writer.write_fixed(&self.octets())?;
        Ok(())
    }
}

impl<K, V, H> Encode for HashMap<K, V, H>
where
    K: Encode,
    V: Encode,
{
    fn produces() -> DataTypeNode {
        DataTypeNode::Map([Box::new(K::produces()), Box::new(V::produces())])
    }

    fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
        let mut writer = writer.write_map()?;

        for (k, v) in self {
            writer.write(k, v)?;
        }

        writer.finish();

        Ok(())
    }

    fn compatible(column_type: &DataTypeNode) -> bool {
        recursive_compatible(column_type, |column_type| {
            let DataTypeNode::Map([key_ty, val_ty]) = column_type else {
                return false;
            };

            K::compatible(key_ty) && V::compatible(val_ty)
        })
    }
}

impl<K, V> Encode for BTreeMap<K, V>
where
    K: Encode,
    V: Encode,
{
    fn produces() -> DataTypeNode {
        DataTypeNode::Map([Box::new(K::produces()), Box::new(V::produces())])
    }

    fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
        let mut writer = writer.write_map()?;

        for (k, v) in self {
            writer.write(k, v)?;
        }

        writer.finish();

        Ok(())
    }

    fn compatible(column_type: &DataTypeNode) -> bool {
        recursive_compatible(column_type, |column_type| {
            let DataTypeNode::Map([key_ty, val_ty]) = column_type else {
                return false;
            };

            K::compatible(key_ty) && V::compatible(val_ty)
        })
    }
}

#[cfg(feature = "uuid")]
mod uuid {
    use super::{Encode, ValueWriter};
    use crate::error::BoxedError;
    use clickhouse_types::DataTypeNode;
    use uuid::Uuid;

    impl Encode for Uuid {
        fn produces() -> DataTypeNode {
            DataTypeNode::UUID
        }

        fn encode(&self, writer: &mut ValueWriter<'_>) -> Result<(), BoxedError> {
            // https://clickhouse.com/docs/interfaces/specs/NativeFormat#uuid
            // Wire bytes 0..7 = canonical bytes 0..7 reversed.
            // Wire bytes 8..15 = canonical bytes 8..15 reversed.
            let (lo_bytes, hi_bytes) = self.as_u64_pair();

            let mut bytes = [0u8; 16];
            bytes[..8].copy_from_slice(&lo_bytes.to_le_bytes());
            bytes[8..].copy_from_slice(&hi_bytes.to_le_bytes());

            writer.write_fixed(&bytes)?;

            Ok(())
        }
    }
}
