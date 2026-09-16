use crate::error::Error;
use crate::native::{Layout, LayoutKind, LayoutLowCardinality};
use clickhouse_types::DataTypeNode;
use std::marker::PhantomData;
use std::ops::Range;
use std::slice;

use crate::native::Decode;
use crate::native::decode::ValueReader;

/// Iterator over data in an `Array(_)` column or subtype.
pub struct ArrayReader<'a, T> {
    elem_type: &'a DataTypeNode,
    kind: IterKind<'a>,
    nulls: Option<slice::Iter<'a, u8>>,
    // covariant over `'a` and `T`
    _marker: PhantomData<&'a T>,
}

/// View into the data for an `Array(_)` column or subtype.
///
/// Decode array elements as a Rust type using [`ArrayData::into_reader()`].
pub struct ArrayData<'a> {
    pub(super) elem_type: &'a DataTypeNode,
    pub(super) layout: &'a Layout,
    pub(super) indices: Range<usize>,
}

/// Iterator over heterogeneous elements of an individual `Tuple(...)` cell.
///
/// As an example, a column `Tuple(UInt32, String, Map<String, String>)` will
/// yield a `UInt32` value, then a `String` value, then the `Map<...>` value.
pub struct TupleIter<'a> {
    pub(super) types: slice::Iter<'a, DataTypeNode>,
    pub(super) layouts: slice::Iter<'a, Layout>,
    pub(super) index: usize,
}

enum IterKind<'a> {
    Fixed(slice::ChunksExact<'a, u8>),
    Variable {
        next_start: usize,
        end_indices: slice::Iter<'a, usize>,
        data: &'a [u8],
    },
    Array {
        // To avoid having to recurse into `DataTypeNode` every time,
        // we memoize what the element type is
        elem_type: &'a DataTypeNode,
        next_start: usize,
        end_indices: slice::Iter<'a, usize>,
        elem_layout: &'a Layout,
    },
    Tuple {
        types: &'a [DataTypeNode],
        layouts: &'a [Layout],
        elem_indices: Range<usize>,
    },
    Map {
        key_ty: &'a DataTypeNode,
        val_ty: &'a DataTypeNode,
        next_start: usize,
        end_indices: slice::Iter<'a, usize>,
        key_val_layouts: &'a [Layout; 2],
    },
    LowCardinality {
        inner_type: &'a DataTypeNode,
        keys: slice::Iter<'a, usize>,
        lc: &'a LayoutLowCardinality,
    },
}

impl<'a> ArrayData<'a> {
    pub(super) fn at_index(elem_type: &'a DataTypeNode, layout: &'a Layout, index: usize) -> Self {
        Self {
            elem_type,
            layout,
            // If this overflows it'll just result in an empty range
            indices: index..index.saturating_add(1),
        }
    }

    /// Validate and begin decoding the array data.
    ///
    /// # Errors
    /// Returns [`Error::SchemaMismatch`] if the element type is not compatible with `T`
    /// according to [`Decode::compatible()`].
    ///
    /// This is an extra sanity check as the outer `Decode` impl should already have validated for
    /// compatibility.
    pub fn into_reader<T>(self) -> Result<ArrayReader<'a, T>, Error>
    where
        T: Decode<'a>,
    {
        if !T::compatible(self.elem_type.remove_compatible_wrappers()) {
            return Err(Error::SchemaMismatch(format!(
                "incompatible data type {}",
                self.elem_type
            )));
        }

        Ok(self.into_reader_unchecked())
    }

    pub(super) fn into_reader_unchecked<T>(self) -> ArrayReader<'a, T>
    where
        T: Decode<'a>,
    {
        ArrayReader {
            elem_type: self.elem_type,
            kind: match self.layout.kind {
                LayoutKind::Fixed {
                    type_width,
                    ref data,
                } => IterKind::Fixed(
                    data[self.indices.start * type_width..self.indices.end * type_width]
                        .chunks_exact(type_width),
                ),
                LayoutKind::Variable {
                    end_offsets: ref end_indices,
                    ref data,
                } => {
                    let next_start = end_indices[..self.indices.start]
                        .last()
                        .copied()
                        .unwrap_or(0);

                    IterKind::Variable {
                        next_start,
                        end_indices: end_indices[self.indices.clone()].iter(),
                        data,
                    }
                }
                LayoutKind::Array {
                    ref elem_layout,
                    ref end_indices,
                } => {
                    let next_start = end_indices[..self.indices.start]
                        .last()
                        .copied()
                        .unwrap_or(0);

                    IterKind::Array {
                        elem_type: unwrap_array_type(self.elem_type).expect("BUG"),
                        next_start,
                        end_indices: end_indices[self.indices.clone()].iter(),
                        elem_layout,
                    }
                }
                LayoutKind::Tuple { ref layouts } => IterKind::Tuple {
                    types: unwrap_tuple_type(self.elem_type).expect("BUG"),
                    layouts,
                    elem_indices: self.indices.clone(),
                },
                LayoutKind::Map {
                    ref key_val_layouts,
                    ref end_indices,
                } => {
                    let [key_ty, val_ty] = unwrap_map_type(self.elem_type).expect("BUG");

                    let next_start = end_indices[..self.indices.start]
                        .last()
                        .copied()
                        .unwrap_or(0);

                    IterKind::Map {
                        key_ty,
                        val_ty,
                        next_start,
                        key_val_layouts,
                        end_indices: end_indices[self.indices.clone()].iter(),
                    }
                }
                LayoutKind::LowCardinality(ref lc) => IterKind::LowCardinality {
                    inner_type: unwrap_lc_type(self.elem_type).expect("BUG"),
                    keys: lc.keys[self.indices.clone()].iter(),
                    lc,
                },
            },
            nulls: self
                .layout
                .nulls
                .as_ref()
                .map(|nulls| nulls[self.indices].iter()),
            _marker: PhantomData,
        }
    }
}

impl<'a, T> Iterator for ArrayReader<'a, T>
where
    T: Decode<'a>,
{
    type Item = Result<T, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        macro_rules! check_null {
            () => {
                match self.nulls.as_mut().map(Iterator::next) {
                    // not-null = 0x0, or no null-map
                    Some(Some(&0)) | None => (),
                    Some(Some(_)) => {
                        return Some(T::decode_null(self.elem_type).map_err(Error::Other))
                    }
                    Some(None) => return None,
                }
            };
        }

        match &mut self.kind {
            IterKind::Fixed(iter) => {
                let native_bytes = iter.next()?;

                // Need to make sure we advance the main iterator first
                check_null!();

                Some(
                    T::decode(&mut ValueReader {
                        data_type: self.elem_type,
                        native_bytes,
                    })
                    .map_err(Error::Other),
                )
            }
            IterKind::Variable {
                next_start,
                end_indices,
                data,
            } => {
                let end = *end_indices.next()?;

                let start = *next_start;
                *next_start = end;

                // Need to make sure we advance the main iterator first
                check_null!();

                Some(
                    T::decode(&mut ValueReader {
                        data_type: self.elem_type,
                        native_bytes: &data[start..end],
                    })
                    .map_err(Error::Other),
                )
            }
            IterKind::Array {
                elem_type,
                next_start,
                end_indices,
                elem_layout,
            } => {
                let array_end = *end_indices.next()?;

                let indices = *next_start..array_end;

                *next_start = array_end;

                Some(
                    T::decode_array(ArrayData {
                        elem_type,
                        layout: elem_layout,
                        indices,
                    })
                    .map_err(Error::Other),
                )
            }
            IterKind::Tuple {
                types,
                layouts,
                elem_indices,
            } => {
                let index = elem_indices.next()?;

                // Need to make sure we advance the main iterator first
                check_null!();

                Some(
                    T::decode_tuple(TupleIter {
                        layouts: layouts.iter(),
                        types: types.iter(),
                        index,
                    })
                    .map_err(Error::Other),
                )
            }
            IterKind::Map {
                key_ty,
                val_ty,
                next_start,
                end_indices,
                key_val_layouts: [key_layout, val_layout],
            } => {
                let array_end = *end_indices.next()?;

                let indices = *next_start..array_end;

                *next_start = array_end;

                Some(
                    T::decode_map(
                        ArrayData {
                            elem_type: key_ty,
                            layout: key_layout,
                            indices: indices.clone(),
                        },
                        ArrayData {
                            elem_type: val_ty,
                            layout: val_layout,
                            indices,
                        },
                    )
                    .map_err(Error::Other),
                )
            }
            IterKind::LowCardinality {
                inner_type,
                keys,
                lc,
            } => {
                let key = *keys.next()?;

                // Check the null map in case of `Nullable(LowCardinality(...))`
                check_null!();

                // `key = 0` is the NULL placeholder
                if lc.is_nullable && key == 0 {
                    return Some(T::decode_null(self.elem_type).map_err(Error::Other));
                }

                Some(
                    ArrayData::at_index(inner_type, &lc.dict, key)
                        .into_reader::<T>()
                        .and_then(|mut reader| {
                            reader.next().ok_or_else(|| {
                                Error::DataFormat(
                                    format!(
                                        "data for LowCardinality({inner_type}) key not found: {key}"
                                    )
                                    .into(),
                                )
                            })?
                        }),
                )
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match &self.kind {
            IterKind::Fixed(chunks) => chunks.size_hint(),
            IterKind::Variable { end_indices, .. } => end_indices.size_hint(),
            IterKind::Array { end_indices, .. } => end_indices.size_hint(),
            IterKind::Tuple { elem_indices, .. } => elem_indices.size_hint(),
            IterKind::Map { end_indices, .. } => end_indices.size_hint(),
            IterKind::LowCardinality { keys, .. } => keys.size_hint(),
        }
    }
}

impl<'a> TupleIter<'a> {
    /// Decode the next element in the tuple.
    ///
    /// For example, a tuple type `Tuple(UInt32, String, Map<String, String>)`
    /// should first decode a `u32`, then a (Rust) `String`, then `HashMap<String, String>`
    /// or `BTreeMap<String, String>`.
    pub fn decode_next<T: Decode<'a>>(&mut self) -> Result<T, Error> {
        let (elem_type, layout) = self.types.next().zip(self.layouts.next()).ok_or_else(|| {
            Error::SchemaMismatch("attempting to decode tuple with more types than received".into())
        })?;

        ArrayData::at_index(elem_type, layout, self.index)
            .into_reader::<T>()?
            .next()
            .ok_or_else(|| {
                Error::SchemaMismatch("attempting to decode from an empty array".into())
            })?
    }
}

fn unwrap_array_type(data_type: &DataTypeNode) -> Result<&DataTypeNode, Error> {
    match data_type {
        DataTypeNode::Array(elem_type) => Ok(elem_type),
        DataTypeNode::Nullable(inner) | DataTypeNode::SimpleAggregateFunction(_, inner) => {
            unwrap_array_type(inner)
        }
        _ => Err(Error::SchemaMismatch(format!(
            "expected Array type, got {data_type}"
        ))),
    }
}

fn unwrap_tuple_type(data_type: &DataTypeNode) -> Result<&[DataTypeNode], Error> {
    match data_type {
        DataTypeNode::Tuple(types) => Ok(types),
        DataTypeNode::Nullable(inner) | DataTypeNode::SimpleAggregateFunction(_, inner) => {
            unwrap_tuple_type(inner)
        }
        _ => Err(Error::SchemaMismatch(format!(
            "expected Tuple type, got {data_type}"
        ))),
    }
}

fn unwrap_map_type(data_type: &DataTypeNode) -> Result<&[Box<DataTypeNode>; 2], Error> {
    match data_type {
        DataTypeNode::Map(types) => Ok(types),
        DataTypeNode::Nullable(inner) | DataTypeNode::SimpleAggregateFunction(_, inner) => {
            unwrap_map_type(inner)
        }
        _ => Err(Error::SchemaMismatch(format!(
            "expected Tuple type, got {data_type}"
        ))),
    }
}

fn unwrap_lc_type(data_type: &DataTypeNode) -> Result<&DataTypeNode, Error> {
    match data_type {
        DataTypeNode::LowCardinality(elem_type) => Ok(elem_type),
        DataTypeNode::Nullable(inner) | DataTypeNode::SimpleAggregateFunction(_, inner) => {
            unwrap_array_type(inner)
        }
        _ => Err(Error::SchemaMismatch(format!(
            "expected LowCardinality type, got {data_type}"
        ))),
    }
}
