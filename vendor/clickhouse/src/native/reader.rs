use crate::error::Error;
use crate::native::string::MaybeUtf8;
use crate::native::utils::type_fixed_width;
use crate::native::varuint::ParseVarUInt;
use crate::native::{Block, Column, Layout, LayoutKind, LayoutLowCardinality};
use crate::response::Chunks;
use bytes::{Buf, Bytes, BytesMut};
use clickhouse_types::DataTypeNode;
use futures_util::StreamExt;
use std::cmp;
use std::collections::VecDeque;
use std::fmt::{Display, Formatter};
use std::ops::ControlFlow;

pub(crate) struct BlockReader {
    inner: ReaderInner,
    // Since versioned type encoding is split into a "prefix phase" and a "data phase"
    // we need to read state prefixes separately and remember what we've seen
    prefix_queue: VecDeque<StatePrefix>,
}

/// Captures errors that may occur while decoding a [Native Format] block.
///
/// [Native Format]: https://clickhouse.com/docs/reference/interfaces/specs/NativeFormat
#[derive(Debug)]
pub struct BlockReadError {
    message: String,
    column_name: Option<MaybeUtf8>,
    column_type: Option<DataTypeNode>,
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

struct ReaderInner {
    chunks: Chunks,
    last_chunk: Bytes,
}

#[derive(Debug)]
enum StatePrefix {
    // LowCardinality state prefix is just `version = 1`, no state necessary
    LowCardinality,
}

#[repr(u64)]
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum LcKeyType {
    UInt8 = 0,
    UInt16,
    UInt32,
    UInt64,
}

macro_rules! read_error {
    ($($format_args:tt)*) => {
        BlockReadError {
            message: format!($($format_args)*),
            column_name: None,
            column_type: None,
            source: None,
        }
    }
}

/// Maximum number of values allowed in any one column.
///
/// This is a hardcoded limit, not expected to be reached by any practical application,
/// meant to guard against allocating based on malicious or malformed data.
///
/// Patched for Navop: upstream hardcodes `1 << 32`, which overflows `usize` on
/// 32-bit targets (e.g. `i686-pc-windows-msvc`) at compile time. On 32-bit
/// `usize::MAX` is the effective ceiling, so fall back to it there.
const SAFE_ALLOCATION_LIMIT: usize = if usize::BITS >= 64 {
    1usize << 32
} else {
    usize::MAX
};

impl BlockReader {
    pub(crate) fn new(chunks: Chunks) -> Self {
        BlockReader {
            inner: ReaderInner {
                chunks,
                last_chunk: Bytes::new(),
            },
            prefix_queue: VecDeque::new(),
        }
    }

    /// **NOT** cancel-safe. Intended to be wrapped in an async task for cancel-safety.
    pub(crate) async fn read_block(&mut self) -> Result<Option<Block>, Error> {
        if !self.inner.try_read_chunk().await? {
            // Native format over HTTP doesn't send a zero-row block on an empty resultset,
            // it just sends nothing at all.
            return Ok(None);
        }

        let num_columns = self.inner.read_varuint().await?;
        let num_rows = self.inner.read_varuint().await?;

        let num_rows = usize::try_from(num_rows)
            .map_err(|_| read_error!("number of rows in block is out of range: {num_rows}"))?;

        if num_rows > SAFE_ALLOCATION_LIMIT {
            return Err(read_error!(
                "number of rows in block exceeds safe limit: {num_rows} vs {SAFE_ALLOCATION_LIMIT}"
            )
            .into());
        }

        if num_columns == 0 {
            return Ok(None);
        }

        let mut columns = Vec::new();

        for col in 0..num_columns {
            columns.push(self.read_column(num_rows).await.inspect_err(|e| {
                tracing::debug!(col, "error reading column: {e:?}");
            })?);
        }

        Ok(Some(Block::from_columns(columns, num_rows)))
    }

    async fn read_column(&mut self, num_rows: usize) -> Result<Column, Error> {
        let name = self.inner.read_string().await?;
        let data_type = self.inner.read_string().await?;

        let data_type = data_type.as_str().ok_or_else(|| {
            read_error!("invalid data type {data_type:?}").with_column(&name, None)
        })?;

        let data_type = DataTypeNode::new(data_type).map_err(|e| {
            read_error!("error parsing data type {data_type:?}")
                .with_column(&name, None)
                .with_source(e)
        })?;

        if num_rows != 0 {
            self.read_state_prefix(&name, &data_type)
                .await
                // so every error message doesn't have to manually mention the column name/type
                .err_with_column(&name, Some(&data_type))?;
        }

        let layout = self
            .read_data(&name, &data_type, num_rows, num_rows != 0)
            .await
            .err_with_column(&name, Some(&data_type))?;

        Ok(Column {
            name,
            data_type,
            layout,
        })
    }

    async fn read_state_prefix(
        &mut self,
        column_name: &MaybeUtf8,
        data_type: &DataTypeNode,
    ) -> Result<(), Error> {
        match data_type {
            DataTypeNode::LowCardinality(_) => {
                let prefix = self.inner.read_int64().await?;

                if prefix != 1 {
                    // sharedDictionariesWithAdditionalKeys
                    return Err(read_error!(
                        "unexpected/unsupported serialization version of LowCardinality: \
                         expected 1, got {prefix}"
                    )
                    .into());
                }

                self.prefix_queue.push_back(StatePrefix::LowCardinality);
            }
            // State prefix for composite types needs to be read _before_ the composite type layout
            DataTypeNode::Array(inner_type) => {
                Box::pin(self.read_state_prefix(column_name, inner_type)).await?;
            }
            DataTypeNode::Tuple(types) => {
                for ty in types {
                    Box::pin(self.read_state_prefix(column_name, ty)).await?;
                }
            }
            DataTypeNode::Map([key_ty, val_ty]) => {
                Box::pin(self.read_state_prefix(column_name, key_ty)).await?;
                Box::pin(self.read_state_prefix(column_name, val_ty)).await?;
            }
            DataTypeNode::JSON
            | DataTypeNode::JsonWithHint(_)
            | DataTypeNode::Variant(_)
            | DataTypeNode::Dynamic => {
                return Err(
                    read_error!("unimplemented deserialization of (sub)type {data_type}").into(),
                );
            }
            _ => (),
        }

        Ok(())
    }

    async fn read_data(
        &mut self,
        column_name: &MaybeUtf8,
        data_type: &DataTypeNode,
        num_values: usize,
        block_nonempty: bool,
    ) -> Result<Layout, Error> {
        if num_values > SAFE_ALLOCATION_LIMIT {
            // Ideally one of the other checks will give the user a more specific error message.
            return Err(read_error!(
                "number of values exceeds safe allocation limit: \
                 {num_values} vs {SAFE_ALLOCATION_LIMIT}"
            )
            .into());
        }

        let (data_type, nulls) = if let DataTypeNode::Nullable(inner) = data_type {
            // Read off the null bitmap
            (&**inner, Some(self.inner.read_bytes(num_values).await?))
        } else {
            (data_type, None)
        };

        if let Some(type_width) = type_fixed_width(data_type) {
            let total_bytes = type_width.checked_mul(num_values).ok_or_else(|| {
                read_error!("data size is too large: {num_values} rows X {type_width} bytes")
            })?;

            return Ok(Layout {
                kind: LayoutKind::Fixed {
                    type_width,
                    data: self.inner.read_bytes(total_bytes).await?,
                },
                nulls,
                num_values,
            });
        }

        match data_type.remove_simple_aggregate_function() {
            DataTypeNode::String => {
                // Assume default growth strategy is fine
                let mut data = BytesMut::new();
                let mut end_offsets = Vec::with_capacity(num_values);

                for row in 0..num_values {
                    let len = self.inner.read_varuint().await?;

                    let len = usize::try_from(len)
                        .map_err(|_| read_error!("string #{row} length {len} is out of range"))?;

                    self.inner.read_bytes_into(len, &mut data).await?;
                    end_offsets.push(data.len());
                }

                Ok(Layout {
                    kind: LayoutKind::Variable {
                        data: data.freeze(),
                        end_offsets: end_offsets.into(),
                    },
                    nulls,
                    num_values,
                })
            }
            DataTypeNode::Array(elem_type) => {
                let end_indices = self.inner.read_array_indices(num_values).await?;

                let total_len = end_indices.last().copied().unwrap_or(0);

                if total_len > SAFE_ALLOCATION_LIMIT {
                    return Err(read_error!(
                        "total length of arrays in column exceeds safe limit: \
                         {total_len} vs {SAFE_ALLOCATION_LIMIT}"
                    )
                    .into());
                }

                // Recursive `async fn` currently requires boxing
                let elem_layout =
                    Box::pin(self.read_data(column_name, elem_type, total_len, block_nonempty))
                        .await?;

                Ok(Layout {
                    kind: LayoutKind::Array {
                        end_indices,
                        elem_layout: Box::new(elem_layout),
                    },
                    nulls,
                    num_values,
                })
            }
            DataTypeNode::Tuple(types) => {
                let mut layouts = Vec::with_capacity(types.len());

                for ty in types {
                    layouts.push(
                        Box::pin(self.read_data(column_name, ty, num_values, block_nonempty))
                            .await?,
                    );
                }

                Ok(Layout {
                    kind: LayoutKind::Tuple {
                        layouts: layouts.into(),
                    },
                    nulls,
                    num_values,
                })
            }
            DataTypeNode::Map([key_ty, val_ty]) => {
                // Maps are just `Array(Tuple(K, V))` but the way this code is structured,
                // it's easier to create a specialized routine to read them
                let end_indices = self.inner.read_array_indices(num_values).await?;

                let total_len = end_indices.last().copied().unwrap_or(0);

                if total_len > SAFE_ALLOCATION_LIMIT {
                    return Err(read_error!(
                        "total number of map entries exceeds safe limit: \
                         {total_len} vs {SAFE_ALLOCATION_LIMIT}"
                    )
                    .into());
                }

                let key_layout =
                    Box::pin(self.read_data(column_name, key_ty, total_len, block_nonempty))
                        .await?;
                let value_layout =
                    Box::pin(self.read_data(column_name, val_ty, total_len, block_nonempty))
                        .await?;

                Ok(Layout {
                    kind: LayoutKind::Map {
                        key_val_layouts: Box::new([key_layout, value_layout]),
                        end_indices,
                    },
                    nulls,
                    num_values,
                })
            }
            DataTypeNode::LowCardinality(inner_type) => {
                Ok(Layout {
                    kind: LayoutKind::LowCardinality(
                        self.read_lc_data(column_name, inner_type, num_values, block_nonempty)
                            .await?,
                    ),
                    // Server disallows `Nullable(LowCardinality(...))` but we don't gain much
                    // from asserting that here
                    nulls,
                    num_values,
                })
            }
            _ => Err(read_error!("data type {data_type:?} not implemented").into()),
        }
    }

    async fn read_lc_data(
        &mut self,
        column_name: &MaybeUtf8,
        inner_type: &DataTypeNode,
        num_rows: usize,
        block_nonempty: bool,
    ) -> Result<LayoutLowCardinality, Error> {
        const HAS_ADDITIONAL_KEYS_BIT: u64 = 0x200;
        const NEEDS_UPDATE_DICTIONARY_BIT: u64 = 0x400;

        // NEED_GLOBAL_DICTIONARY_BIT (0x800) should NEVER be set
        const VALID_FLAG_BITS: u64 = HAS_ADDITIONAL_KEYS_BIT | NEEDS_UPDATE_DICTIONARY_BIT;

        // `LowCardinality(Nullable)` doesn't emit a null map, instead `dict[0]` is the null value
        let (non_nullable, is_nullable) = if let DataTypeNode::Nullable(inner) = inner_type {
            (&**inner, true)
        } else {
            (inner_type, false)
        };

        // Array(LowCardinality(...)) still emits a prefix even if the array itself is empty
        if block_nonempty {
            // Ensure that we read our version prefix even if it doesn't tell us anything
            match self.prefix_queue.pop_front() {
                Some(StatePrefix::LowCardinality) => (),
                other => {
                    return Err(read_error!(
                        "error reading (sub)type LowCardinality({inner_type}): \
                         expected state prefix 0x01, got {other:?}"
                    )
                    .into());
                }
            }
        }

        if num_rows == 0 {
            return Ok(LayoutLowCardinality {
                keys: Box::new([]),
                dict: Box::new(
                    // The easiest way to implement this is to delegate back to `read_type_data()`
                    Box::pin(self.read_data(column_name, non_nullable, num_rows, block_nonempty))
                        .await?,
                ),
                is_nullable,
            });
        }

        let metadata = self.inner.read_uint64().await?;

        let key_type = LcKeyType::try_from_metadata(metadata)?;

        let flags = metadata & !LcKeyType::MASK;

        if flags != VALID_FLAG_BITS {
            return Err(read_error!(
                "(sub)type LowCardinality({inner_type}) has \
                 invalid or unexpected metadata bits (metadata: {metadata:X}); \
                 expected: {VALID_FLAG_BITS:X}, received: {flags:X}",
            )
            .into());
        }

        let dict_len = self.inner.read_uint64().await?;
        let dict_len = usize::try_from(dict_len).map_err(|_| {
            read_error!("LowCardinality({inner_type}) dictionary size too large: {dict_len}")
        })?;

        // called from `self.read_type_data()` so we have to box
        let dict = Box::pin(self.read_data(column_name, non_nullable, dict_len, block_nonempty))
            .await
            .map_err(|e| {
                read_error!("error reading LowCardinality({inner_type}) dictionary").with_source(e)
            })?;

        let keys_len = self.inner.read_uint64().await?;
        let keys_len = usize::try_from(keys_len).map_err(|_| {
            read_error!("LowCardinality({inner_type}) keys count too large: {keys_len}")
        })?;

        if keys_len != num_rows {
            return Err(read_error!(
                "LowCardinality({inner_type}) keys count does not match number of rows in block: \
                 {keys_len} vs {num_rows}; this likely means a bug or corrupted data"
            )
            .into());
        }

        let keys = self.inner.read_lc_keys(key_type, num_rows).await?;

        Ok(LayoutLowCardinality {
            keys,
            dict: Box::new(dict),
            is_nullable,
        })
    }
}

impl ReaderInner {
    async fn read_string(&mut self) -> Result<MaybeUtf8, Error> {
        let len = self.read_varuint().await?;

        let len =
            usize::try_from(len).map_err(|_| read_error!("string length too large: {len}"))?;

        if len > SAFE_ALLOCATION_LIMIT {
            return Err(read_error!(
                "string size exceeds safe allocation limit: {len} vs {SAFE_ALLOCATION_LIMIT}"
            )
            .into());
        }

        Ok(self.read_bytes(len).await?.into())
    }

    async fn read_varuint(&mut self) -> Result<u64, Error> {
        let mut parser = ParseVarUInt::default();

        loop {
            if let ControlFlow::Break(val) = parser
                .feed(&mut self.last_chunk)
                .map_err(|e| read_error!("error parsing VarUInt").with_source(e))?
            {
                return Ok(val);
            }

            self.read_chunk().await?;
        }
    }

    async fn read_array_indices(&mut self, num_rows: usize) -> Result<Box<[usize]>, Error> {
        let mut end_indices = Vec::with_capacity(num_rows);

        for _ in 0..num_rows {
            // Note: cumulative length, last value is total number of elements
            let length = self.read_uint64().await?;

            let length = usize::try_from(length)
                .map_err(|_| read_error!("array length out of range: {length}"))?;

            end_indices.push(length);
        }

        Ok(end_indices.into())
    }

    async fn read_lc_keys(
        &mut self,
        key_type: LcKeyType,
        num_rows: usize,
    ) -> Result<Box<[usize]>, Error> {
        let mut keys = Vec::with_capacity(num_rows);

        for i in 0..num_rows {
            let mut buf = [0u8; LcKeyType::MAX_BYTE_WIDTH];

            self.fill_buf(&mut buf[..key_type.byte_width()]).await?;

            let key = u64::from_le_bytes(buf);

            let key = usize::try_from(key).map_err(|_| {
                Error::DataFormat(
                    format!("LowCardinality dictionary key out of range at index {i}: {key}")
                        .into(),
                )
            })?;

            keys.push(key);
        }

        Ok(keys.into())
    }

    async fn read_bytes(&mut self, len: usize) -> Result<Bytes, Error> {
        if len == 0 {
            return Ok(Bytes::new());
        }

        self.read_chunk().await?;

        if self.last_chunk.len() >= len {
            return Ok(self.last_chunk.split_to(len));
        }

        let mut buf = BytesMut::with_capacity(len);
        self.read_bytes_into(len, &mut buf).await?;

        Ok(buf.freeze())
    }

    async fn read_bytes_into(&mut self, mut amt: usize, buf: &mut BytesMut) -> Result<(), Error> {
        let expected_len = buf.len().saturating_add(amt);

        if expected_len > SAFE_ALLOCATION_LIMIT {
            return Err(read_error!(
                "safe allocation limit exceeded: {expected_len} vs {SAFE_ALLOCATION_LIMIT}"
            )
            .into());
        }

        while amt > 0 {
            self.read_chunk().await?;

            let read_len = cmp::min(self.last_chunk.len(), amt);

            buf.extend_from_slice(&self.last_chunk[..read_len]);

            self.consume(read_len);

            amt -= read_len;
        }

        Ok(())
    }

    async fn read_bytes_fixed<const LEN: usize>(&mut self) -> Result<[u8; LEN], Error> {
        let mut buf = [0u8; LEN];

        self.fill_buf(&mut buf).await?;

        Ok(buf)
    }

    async fn fill_buf(&mut self, mut buf: &mut [u8]) -> Result<(), Error> {
        while !buf.is_empty() {
            self.read_chunk().await?;

            let read_len = cmp::min(buf.len(), self.last_chunk.len());

            let dst = buf
                .split_off_mut(..read_len)
                .expect("BUG: we just validated `read_len` is in bounds");

            dst.copy_from_slice(&self.last_chunk[..read_len]);

            self.consume(read_len);
        }

        Ok(())
    }

    async fn read_uint64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.read_bytes_fixed().await?))
    }

    async fn read_int64(&mut self) -> Result<i64, Error> {
        Ok(i64::from_le_bytes(self.read_bytes_fixed().await?))
    }

    async fn read_chunk(&mut self) -> Result<(), Error> {
        if !self.try_read_chunk().await? {
            tracing::trace!("error: not enough data");
            return Err(Error::NotEnoughData);
        }

        Ok(())
    }

    async fn try_read_chunk(&mut self) -> Result<bool, Error> {
        if self.last_chunk.is_empty() {
            let Some(chunk) = self.chunks.next().await else {
                return Ok(false);
            };

            self.last_chunk = chunk?.data;
        }

        Ok(true)
    }

    fn consume(&mut self, len: usize) {
        self.last_chunk.advance(len);

        // `Bytes::advance()` doesn't decrement the refcount when it's empty,
        // preventing the buffer from being reused
        if self.last_chunk.is_empty() {
            self.last_chunk = Bytes::new();
        }
    }
}

impl LcKeyType {
    const MASK: u64 = 0xFF; // Low 8 bits

    const MAX_BYTE_WIDTH: usize = Self::UInt64.byte_width();

    fn try_from_metadata(metadata: u64) -> Result<Self, Error> {
        Ok(match metadata & Self::MASK {
            0 => Self::UInt8,
            1 => Self::UInt16,
            2 => Self::UInt32,
            3 => Self::UInt64,
            unknown => {
                return Err(read_error!("unknown LowCardinality key type: {unknown}").into());
            }
        })
    }

    const fn byte_width(&self) -> usize {
        match self {
            LcKeyType::UInt8 => 1,
            LcKeyType::UInt16 => 2,
            LcKeyType::UInt32 => 4,
            LcKeyType::UInt64 => 8,
        }
    }
}

impl Display for BlockReadError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "error reading native block")?;

        match (&self.column_name, &self.column_type) {
            (Some(col_name), Some(col_type)) => {
                write!(f, " at column `{col_name} {col_type}`")?;
            }
            (Some(col_name), None) => {
                write!(f, " at column `{col_name}`")?;
            }
            _ => (),
        }

        write!(f, ": {}", self.message)
    }
}

impl std::error::Error for BlockReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // `self.source.as_deref()` on its own doesn't trigger the correct coercion
        Some(self.source.as_deref()?)
    }
}

impl BlockReadError {
    fn with_column(mut self, name: &MaybeUtf8, ty: Option<&DataTypeNode>) -> Self {
        self.set_column(name, ty);
        self
    }

    fn set_column(&mut self, name: &MaybeUtf8, ty: Option<&DataTypeNode>) {
        self.column_name = Some(name.clone());
        self.column_type = ty.cloned();
    }

    fn with_source(
        mut self,
        source: impl Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
    ) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Get the name of the column that was being read when the error occurred.
    ///
    /// Returns `None` if a column was not being read yet, or if the column name is not UTF-8.
    ///
    /// See [`Self::column_name_bytes()`] if handling column names that may not be UTF-8.
    pub fn column_name(&self) -> Option<&str> {
        self.column_name.as_ref().and_then(MaybeUtf8::as_str)
    }

    /// Get the name of the column that was being read when the error occurred.
    ///
    /// Returns `None` if a column was not being read yet.
    pub fn column_name_bytes(&self) -> Option<&[u8]> {
        self.column_name.as_ref().map(MaybeUtf8::as_bytes)
    }
}

trait ResultExt {
    fn err_with_column(self, name: &MaybeUtf8, data_type: Option<&DataTypeNode>) -> Self;
}

impl<T> ResultExt for Result<T, crate::Error> {
    fn err_with_column(self, name: &MaybeUtf8, data_type: Option<&DataTypeNode>) -> Self {
        let Err(e) = self else {
            return self;
        };

        let Error::DataFormat(e) = e else {
            return Err(e);
        };

        Err(Error::DataFormat(
            e.downcast::<BlockReadError>().map_or_else(
                |e| e,
                |mut e| {
                    e.set_column(name, data_type);
                    e
                },
            ),
        ))
    }
}

impl<T> ResultExt for Result<T, BlockReadError> {
    fn err_with_column(self, name: &MaybeUtf8, data_type: Option<&DataTypeNode>) -> Self {
        self.map_err(|e| e.with_column(name, data_type))
    }
}
