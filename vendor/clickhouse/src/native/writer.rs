use crate::Client;
use crate::error::Error;
use crate::insert_formatted::InsertFormatted;
use crate::native::{Block, Column, Layout, LayoutKind, varuint};
use bytes::{BufMut, Bytes, BytesMut};
use std::num::Saturating;

pub(crate) struct BlockWriter {
    insert: InsertFormatted,
    buf: BytesMut,
    sent_rows: Saturating<usize>,
}

impl BlockWriter {
    pub(crate) fn new(insert: InsertFormatted) -> Self {
        Self {
            insert,
            buf: BytesMut::with_capacity(8192),
            sent_rows: Saturating(0),
        }
    }

    pub(crate) fn insert_mut(&mut self) -> &mut InsertFormatted {
        &mut self.insert
    }

    #[cfg_attr(not(test), expect(unused))]
    pub(crate) fn expect_client(&self) -> &Client {
        self.insert.expect_client()
    }

    pub(crate) fn expect_client_mut(&mut self) -> &mut Client {
        self.insert.expect_client_mut()
    }

    pub(crate) async fn write(&mut self, block: &Block) -> Result<(), Error> {
        if block.num_rows() == 0 {
            return Err(Error::Other("attempting to write an empty block".into()));
        }

        // If canceled while writing a block, we have no way to recover.
        // We have to abort the request instead.
        let mut guard = WriteGuard {
            insert: &mut self.insert,
            buf: &mut self.buf,
            finished: false,
        };

        varuint::write(&mut guard.buf, block.columns.len());
        varuint::write(&mut guard.buf, block.num_rows);

        for column in &block.columns {
            guard.write_column(column).await?;
        }

        // We deliberately don't send the block header right away
        // since the first column header can share the same buffer.
        //
        // If `write_column()` wrote any data, this should be a no-op.
        guard.flush().await?;

        guard.finished = true;

        self.sent_rows += block.num_rows;

        Ok(())
    }

    pub(crate) async fn end(mut self) -> Result<(), Error> {
        let mut guard = WriteGuard {
            insert: &mut self.insert,
            buf: &mut self.buf,
            finished: false,
        };

        guard.flush().await?;

        guard.finished = true;

        drop(guard);

        tracing::record_all!(
            self.insert.span(),
            clickhouse.request.sent_rows = self.sent_rows.0,
        );

        self.insert.end().await
    }
}

struct WriteGuard<'a> {
    insert: &'a mut InsertFormatted,
    buf: &'a mut BytesMut,
    finished: bool,
}

impl WriteGuard<'_> {
    async fn write_column(&mut self, column: &Column) -> Result<(), Error> {
        varuint::write(&mut self.buf, column.name.len());
        self.buf.extend_from_slice(column.name.as_bytes());

        // We have to format the data type to know exactly how long the string is,
        // though in most cases we can use a static string.
        let data_type_str = column.data_type.to_str();

        varuint::write(&mut self.buf, data_type_str.len());
        self.buf.extend_from_slice(data_type_str.as_bytes());

        self.write_layout(&column.layout).await
    }

    async fn write_layout(&mut self, layout: &Layout) -> Result<(), Error> {
        if let Some(nulls) = &layout.nulls {
            self.send(nulls).await?;
        }

        match &layout.kind {
            LayoutKind::Fixed { data, .. } => {
                self.send(data).await?;
            }
            LayoutKind::Variable { end_offsets, data } => {
                let mut start_offset = 0;

                // Convert from offsets *back* to lengths and subslices
                for &end_offset in end_offsets {
                    let len = end_offset
                        .checked_sub(start_offset)
                        .ok_or_else(|| Error::Other(format!("BUG: string length underflow in encoding block: {end_offset} - {start_offset}").into()))?;

                    varuint::write(&mut self.buf, len);

                    self.send(&data.slice(start_offset..end_offset)).await?;

                    start_offset = end_offset;
                }
            }
            LayoutKind::LowCardinality(_) => {
                return Err(Error::Other(
                    "inserting LowCardinality data not yet implemented".into(),
                ));
            }
            LayoutKind::Array {
                end_indices,
                elem_layout,
            } => {
                for index in end_indices {
                    self.buf.put_u64_le(u64::try_from(*index).map_err(|_| {
                        Error::Other(format!("array end index out of range: {index}").into())
                    })?);
                }

                Box::pin(self.write_layout(elem_layout)).await?;
            }
            LayoutKind::Tuple { layouts } => {
                for layout in layouts {
                    Box::pin(self.write_layout(layout)).await?;
                }
            }
            LayoutKind::Map {
                key_val_layouts,
                end_indices,
            } => {
                for index in end_indices {
                    self.buf.put_u64_le(u64::try_from(*index).map_err(|_| {
                        Error::Other(format!("map end index out of range: {index}").into())
                    })?);
                }

                Box::pin(self.write_layout(&key_val_layouts[0])).await?;
                Box::pin(self.write_layout(&key_val_layouts[1])).await?;
            }
        }

        Ok(())
    }

    async fn send(&mut self, data: &Bytes) -> Result<(), Error> {
        /// If a data buffer is smaller than this threshold, copy it to `self.buf` instead.
        const COPY_THRESHOLD: usize = 128;

        if data.len() < COPY_THRESHOLD {
            self.buf.extend_from_slice(data);
            Ok(())
        } else {
            self.flush().await?;

            self.insert.send(data.clone()).await
        }
    }

    async fn flush(&mut self) -> Result<(), Error> {
        if !self.buf.is_empty() {
            self.insert.send(self.buf.split().freeze()).await?;
        }

        Ok(())
    }
}

impl Drop for WriteGuard<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.insert.abort();
        }
    }
}
