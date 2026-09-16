#![doc=include_str!("mod.md")]
#![deny(missing_docs)]

use crate::error::Error;
use crate::native::string::MaybeUtf8;
use bytes::Bytes;
use std::ops::Index;

use hashbrown::HashMap;

pub use array::{ArrayData, ArrayReader};
pub use reader::BlockReadError;

use crate::native::decode::Decode;
pub use clickhouse_types::DataTypeNode;

pub(crate) mod array;
pub mod builder;
pub mod decode;
pub mod encode;
pub(crate) mod reader;
pub(crate) mod string;
mod utils;
mod varuint;
pub(crate) mod writer;

#[cfg(test)]
mod tests;

/// A block of data in columnar format.
pub struct Block {
    column_names: HashMap<MaybeUtf8, usize>,
    columns: Box<[Column]>,
    num_rows: usize,
}

impl Block {
    fn from_columns(columns: Vec<Column>, num_rows: usize) -> Self {
        Self {
            column_names: columns
                .iter()
                .enumerate()
                .map(|(i, column)| (column.name.clone(), i))
                .collect(),
            columns: columns.into(),
            num_rows,
        }
    }

    /// The number of rows in this block.
    ///
    /// Note that the size of a single block in a query resultset can be influenced by many things
    /// and does not necessarily represent the full result.
    pub fn num_rows(&self) -> usize {
        self.num_rows
    }

    /// The list of columns in the order returned by the server.
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// Look up a column by name, returning `None` if it is not found (note: case-sensitive).
    pub fn column_by_name(&self, name: &str) -> Option<&Column> {
        // Requires `hashbrown` for the `Equivalent` trait
        let idx = *self.column_names.get(name)?;

        Some(&self.columns[idx])
    }
}

impl Index<&str> for Block {
    type Output = Column;

    /// Look up a [`Column`] by name (note: case-sensitive).
    ///
    /// # Panics
    /// If a column with this name does not exist.
    fn index(&self, name: &str) -> &Self::Output {
        self.column_by_name(name)
            .unwrap_or_else(|| panic!("block does not contain a column with the name {name:?}"))
    }
}

impl Index<usize> for Block {
    type Output = Column;

    /// Look up a [`Column`] by ordinal (zero-based).
    ///
    /// # Panics
    /// If the ordinal is out of bounds.
    ///
    /// For a non-panicking version, use `block.columns().get(index)` instead.
    fn index(&self, index: usize) -> &Self::Output {
        &self.columns[index]
    }
}

/// A single column of data in a [`Block`].
///
/// Decode with [`Column::iter()`].
pub struct Column {
    name: MaybeUtf8,
    data_type: DataTypeNode,
    layout: Layout,
}

struct Layout {
    kind: LayoutKind,
    nulls: Option<Bytes>,
    // Equivalent to `num_rows` for top-level values, but may be different for nested values
    num_values: usize,
}

enum LayoutKind {
    /// Fixed layout. Width of each cell depends only on [`DataTypeNode`].
    Fixed {
        type_width: usize,
        data: Bytes,
    },
    /// Variable-length data (namely strings)
    Variable {
        /// Ending offset of each string in `data`.
        ///
        /// The offset of the first string is always `0` unless this is empty.
        end_offsets: Box<[usize]>,
        // Each `Bytes` instance is 4 `usizes` (32 bytes on 64-bit),
        // so we save 24 bytes per string by linearizing the string data and storing offsets,
        // assuming many small strings instead of fewer big ones, which also amortizes allocations
        data: Bytes,
    },
    /// Layout determined by `LowCardinality` metadata.
    LowCardinality(LayoutLowCardinality),
    /// Array data. Element data governed by `elem_layout`.
    Array {
        /// Ending index of each array in `elem_layout`.
        end_indices: Box<[usize]>,
        elem_layout: Box<Layout>,
    },
    Tuple {
        layouts: Box<[Layout]>,
    },
    Map {
        key_val_layouts: Box<[Layout; 2]>,
        end_indices: Box<[usize]>,
    },
}

struct LayoutLowCardinality {
    keys: Box<[usize]>,
    dict: Box<Layout>,
    /// If `true`, `key = 0` should decode as `NULL` instead of the placeholder value given.
    is_nullable: bool,
}

impl Column {
    /// Get the column name as returned by the server.
    ///
    /// # Panics
    /// If the column name is not valid UTF-8.
    ///
    /// This is unlikely to happen if you have control over the column set returned by the query.
    ///
    /// Use [`Self::name_bytes()`] if panicking is unacceptable.
    pub fn name(&self) -> &str {
        self.name
            .as_str()
            .unwrap_or_else(|| panic!("column name {:?} is not valid UTF-8", self.name))
    }

    /// Get the raw column name as returned by the server.
    pub fn name_bytes(&self) -> &[u8] {
        self.name.as_bytes()
    }

    /// Get the parsed data type of this column.
    pub fn data_type(&self) -> &DataTypeNode {
        &self.data_type
    }

    /// Decode the data of this column as the given type.
    ///
    /// # Errors
    /// Returns [`Error::SchemaMismatch`] if the type is not compatible with the column type
    /// (as determined by [`Decode::compatible()`]).
    ///
    /// If the column data type is `LowCardinality(_)` or `SimpleAggregateFunction(_)`,
    /// the outer type is removed before the compatibility check.
    pub fn iter<'a, T: Decode<'a>>(&'a self) -> Result<ColumnIter<'a, T>, Error> {
        if !T::compatible(self.data_type.remove_compatible_wrappers()) {
            return Err(Error::SchemaMismatch(format!(
                "incompatible data type {:?} of column {:?}",
                self.data_type, self.name
            )));
        }

        Ok(ColumnIter {
            column: self,
            iter: ArrayData {
                elem_type: &self.data_type,
                layout: &self.layout,
                indices: 0..self.layout.num_values,
            }
            .into_reader_unchecked(), // we already checked above with a more specific error
        })
    }
}

/// Typed iterator over data in a [`Column`].
pub struct ColumnIter<'a, T> {
    column: &'a Column,
    iter: ArrayReader<'a, T>,
}

impl<'a, T: 'a> Iterator for ColumnIter<'a, T>
where
    T: Decode<'a>,
{
    type Item = Result<T, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a, T> ColumnIter<'a, T> {
    /// Get the source column of this iterator.
    pub fn column(&self) -> &'a Column {
        self.column
    }
}
