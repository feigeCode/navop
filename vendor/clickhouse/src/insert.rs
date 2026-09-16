use crate::insert_formatted::{BufInsertFormatted, InsertFormatted};
use crate::row_metadata::RowMetadata;
use crate::rowbinary::{serialize_row_binary, serialize_with_validation};
use crate::{
    Client, RowWrite,
    error::Result,
    formats,
    row::{self, Row},
};
use clickhouse_types::put_rbwnat_columns_header;
use std::num::Saturating;
use std::{future::Future, marker::PhantomData, time::Duration};

// The desired max frame size.
const BUFFER_SIZE: usize = 256 * 1024;
// Threshold to send a chunk. Should be slightly less than `BUFFER_SIZE`
// to avoid extra reallocations in case of a big last row.
const MIN_CHUNK_SIZE: usize = const {
    // to use the whole buffer's capacity
    assert!(BUFFER_SIZE.is_power_of_two());
    BUFFER_SIZE - 2048
};

/// Performs one `INSERT`.
///
/// The [`Insert::end`] must be called to finalize the `INSERT`.
/// Otherwise, the whole `INSERT` will be aborted.
///
/// Rows are sent progressively to spread network load.
///
/// # Note: Metadata is Cached
/// If [validation is enabled][Client::with_validation],
/// this helper will query the metadata for the target table to learn the column names and types.
///
/// To avoid querying this metadata every time, it is cached within the [`Client`].
///
/// Any concurrent changes to the table schema may cause insert failures if the metadata
/// is no longer correct. For correct functioning, call [`Client::clear_cached_metadata()`]
/// after any changes to the current database schema.
#[must_use]
pub struct Insert<T> {
    insert: BufInsertFormatted,
    row_metadata: Option<RowMetadata>,
    sent_rows: Saturating<u64>,
    _marker: PhantomData<fn() -> T>, // TODO: test contravariance.
}

impl<T> Insert<T> {
    pub(crate) fn new(client: &Client, table: &str, row_metadata: Option<RowMetadata>) -> Self
    where
        T: Row,
    {
        let fields = row::join_column_names::<T>()
            .expect("the row type must be a struct or a wrapper around it");

        let format = if row_metadata.is_some() {
            formats::ROW_BINARY_WITH_NAMES_AND_TYPES
        } else {
            formats::ROW_BINARY
        };
        let sql = format!("INSERT INTO {table}({fields}) FORMAT {format}");

        Self {
            insert: InsertFormatted::new(client, sql, Some(table))
                .buffered_with_capacity(BUFFER_SIZE),
            row_metadata,
            sent_rows: Saturating(0),
            _marker: PhantomData,
        }
    }

    /// Sets timeouts for different operations.
    ///
    /// `send_timeout` restricts time on sending a data chunk to a socket.
    /// `None` disables the timeout, it's a default.
    /// It's roughly equivalent to `tokio::time::timeout(insert.write(...))`.
    ///
    /// `end_timeout` restricts time on waiting for a response from the CH
    /// server. Thus, it includes all work needed to handle `INSERT` by the
    /// CH server, e.g. handling all materialized views and so on.
    /// `None` disables the timeout, it's a default.
    /// It's roughly equivalent to `tokio::time::timeout(insert.end(...))`.
    ///
    /// These timeouts are much more performant (~x10) than wrapping `write()`
    /// and `end()` calls into `tokio::time::timeout()`.
    pub fn with_timeouts(
        mut self,
        send_timeout: Option<Duration>,
        end_timeout: Option<Duration>,
    ) -> Self {
        self.set_timeouts(send_timeout, end_timeout);
        self
    }

    /// Configure the [roles] to use when executing `INSERT` statements.
    ///
    /// Overrides any roles previously set by this method, [`Insert::with_setting`],
    /// [`Client::with_roles`] or [`Client::with_setting`].
    ///
    /// An empty iterator may be passed to clear the set roles.
    ///
    /// [roles]: https://clickhouse.com/docs/operations/access-rights#role-management
    ///
    /// # Panics
    /// If called after the request is started, e.g., after [`Insert::write`].
    pub fn with_roles(mut self, roles: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.insert.expect_client_mut().set_roles(roles);
        self
    }

    /// Clear any explicit [roles] previously set on this `Insert` or inherited from [`Client`].
    ///
    /// Overrides any roles previously set by [`Insert::with_roles`], [`Insert::with_setting`],
    /// [`Client::with_roles`] or [`Client::with_setting`].
    ///
    /// [roles]: https://clickhouse.com/docs/operations/access-rights#role-management
    ///
    /// # Panics
    /// If called after the request is started, e.g., after [`Insert::write`].
    pub fn with_default_roles(mut self) -> Self {
        self.insert.expect_client_mut().clear_roles();
        self
    }

    /// Similar to [`Client::with_option`], but for this particular INSERT
    /// statement only.
    ///
    /// # Panics
    /// If called after the request is started, e.g., after [`Insert::write`].
    #[track_caller]
    #[deprecated(since = "0.14.3", note = "please use `with_setting` instead")]
    pub fn with_option(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.insert.expect_client_mut().set_setting(name, value);
        self
    }

    /// Similar to [`Client::with_setting`], but for this particular INSERT
    /// statement only.
    ///
    /// # Panics
    /// If called after the request is started, e.g., after [`Insert::write`].
    #[track_caller]
    pub fn with_setting(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.insert.expect_client_mut().set_setting(name, value);
        self
    }

    pub(crate) fn set_timeouts(
        &mut self,
        send_timeout: Option<Duration>,
        end_timeout: Option<Duration>,
    ) {
        self.insert.set_timeouts(send_timeout, end_timeout);
    }

    /// Serializes the provided row into an internal buffer.
    /// Once the buffer is full, it's sent to a background task writing to the
    /// socket.
    ///
    /// Close to:
    ///
    /// ```ignore
    /// async fn write<T>(&self, row: &T) -> Result<usize>;
    /// ```
    ///
    /// A returned future doesn't depend on the row's lifetime.
    ///
    /// Returns an error if the row cannot be serialized or the background task
    /// failed. Once failed, the whole `INSERT` is aborted and cannot be
    /// used anymore.
    ///
    /// # Panics
    ///
    /// If called after the previous call that returned an error.
    pub fn write<'a>(
        &'a mut self,
        row: &T::Value<'_>,
    ) -> impl Future<Output = Result<()>> + 'a + Send
    where
        T: RowWrite,
    {
        let result = self.do_write(row);

        async move {
            result?;
            if self.insert.buf_len() >= MIN_CHUNK_SIZE {
                self.insert.flush().await?;
            }

            self.sent_rows += 1;

            Ok(())
        }
    }

    /// Returns the number of bytes written, not including the RBWNAT header.
    #[inline(always)]
    pub(crate) fn do_write(&mut self, row: &T::Value<'_>) -> Result<usize>
    where
        T: RowWrite,
    {
        // We don't want to wait for the buffer to be full before we start the request,
        // in the event of an error.
        self.init_request_if_required()?;

        // The following calls need an `impl BufMut`
        let buffer = self.insert.buffer_mut();

        let old_buf_size = buffer.len();
        let result = match &self.row_metadata {
            Some(metadata) => serialize_with_validation(&mut *buffer, row, metadata),
            None => serialize_row_binary(&mut *buffer, row),
        };
        let written = buffer.len() - old_buf_size;

        if let Err(e) = &result {
            e.record_in_current_span("error serializing row");
            self.abort();
        }

        result.and(Ok(written))
    }

    /// Ends `INSERT`, the server starts processing the data.
    ///
    /// Succeeds if the server returns 200, that means the `INSERT` was handled
    /// successfully, including all materialized views and quorum writes.
    ///
    /// NOTE: If it isn't called, the whole `INSERT` is aborted.
    pub async fn end(mut self) -> Result<()> {
        // `InsertFormatted::end()` will add `sent_bytes` and `encoded_bytes` to the span.
        tracing::record_all!(
            self.insert._priv_span(),
            clickhouse.request.sent_rows = self.sent_rows.0,
        );

        self.insert.end().await
    }

    fn init_request_if_required(&mut self) -> Result<()> {
        let fresh_request = self.insert.init_request_if_required()?;

        if fresh_request && let Some(metadata) = &self.row_metadata {
            put_rbwnat_columns_header(&metadata.columns, self.insert.buffer_mut())
                .inspect_err(|_| self.abort())?;
        }

        Ok(())
    }

    fn abort(&mut self) {
        self.insert.abort();
    }
}
