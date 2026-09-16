use crate::headers::{with_authentication, with_request_headers};
use crate::{
    Client, Compression,
    error::{Error, Result},
    request_body::{ChunkSender, RequestBody},
    response::Response,
    settings,
};
use bytes::{Bytes, BytesMut};
use hyper::{self, Request};
use std::ops::ControlFlow;
use std::task::{Context, Poll, ready};
use std::{cmp, future::Future, io, mem, panic, pin::Pin, time::Duration};
use tokio::io::AsyncWrite;
use tokio::{
    task::JoinHandle,
    time::{Instant, Sleep},
};
use url::Url;

#[cfg(any(feature = "lz4", feature = "zstd"))]
pub use compression::CompressedData;

// The desired max frame size.
const BUFFER_SIZE: usize = 256 * 1024;

/// Performs one `INSERT`, sending pre-formatted data.
///
/// The [`InsertFormatted::end`] method must be called to finalize the `INSERT`.
/// Otherwise, the whole `INSERT` will be aborted.
///
/// Rows are sent progressively to spread network load.
///
/// # Note: Not Validated
/// Unlike [`Insert`][crate::insert::Insert] and [`Inserter`][crate::inserter::Inserter],
/// this does not perform any validation on the submitted data.
///
/// Only the use of self-describing formats (e.g. CSV, TabSeparated, JSON) is recommended.
///
/// See the [list of supported formats](https://clickhouse.com/docs/interfaces/formats)
/// for details.
#[must_use]
pub struct InsertFormatted {
    state: InsertState,
    #[cfg(any(feature = "lz4", feature = "zstd"))]
    compression: Compression,
    send_timeout: Option<Timeout>,
    end_timeout: Option<Timeout>,
    // Use boxed `Sleep` to reuse a timer entry, it improves performance.
    // Also, `tokio::time::timeout()` significantly increases a future's size.
    sleep: Option<Pin<Box<Sleep>>>,
    span: tracing::Span,
}

struct Timeout {
    duration: Duration,
    is_set: bool,
}

enum InsertState {
    NotStarted {
        client: Box<Client>,
        sql: String,
    },
    Active {
        sender: ChunkSender,
        handle: JoinHandle<Result<()>>,
        sent_bytes: u64,
        encoded_bytes: u64,
    },
    Terminated {
        handle: JoinHandle<Result<()>>,
    },
    Completed,
}

impl InsertState {
    #[inline(always)]
    fn is_not_started(&self) -> bool {
        matches!(self, Self::NotStarted { .. })
    }

    fn sender(&mut self) -> Option<&mut ChunkSender> {
        match self {
            InsertState::Active { sender, .. } => Some(sender),
            _ => None,
        }
    }

    fn handle(&mut self) -> Option<&mut JoinHandle<Result<()>>> {
        match self {
            InsertState::Active { handle, .. } | InsertState::Terminated { handle } => Some(handle),
            _ => None,
        }
    }

    fn client_with_sql(&self) -> Option<(&Client, &str)> {
        match self {
            InsertState::NotStarted { client, sql } => Some((client, sql)),
            _ => None,
        }
    }

    fn expect_client(&self) -> &Client {
        let Self::NotStarted { client, .. } = self else {
            panic!("cannot access client settings while an insert is in-progress")
        };

        client
    }

    #[inline]
    fn expect_client_mut(&mut self) -> &mut Client {
        let Self::NotStarted { client, .. } = self else {
            panic!("cannot modify client settings while an insert is in-progress")
        };

        client
    }

    fn terminated(&mut self, span: &tracing::Span) {
        match mem::replace(self, InsertState::Completed) {
            InsertState::NotStarted { .. } | InsertState::Completed => (),
            InsertState::Active {
                handle,
                sent_bytes,
                encoded_bytes,
                ..
            } => {
                *self = InsertState::Terminated { handle };

                tracing::record_all!(
                    span,
                    clickhouse.request.sent_bytes = sent_bytes,
                    clickhouse.request.encoded_bytes = encoded_bytes,
                );
            }
            InsertState::Terminated { handle } => {
                *self = InsertState::Terminated { handle };
            }
        }
    }
}

impl InsertFormatted {
    pub(crate) fn new(client: &Client, sql: String, collection_name: Option<&str>) -> Self {
        // https://opentelemetry.io/docs/specs/semconv/db/sql/
        // TODO: write our own Semantic Conventions for ClickHouse
        Self {
            span: tracing::info_span!(
                "clickhouse.insert",
                // OTel conventional fields
                // Note that `Empty` or `Option::None` fields are not reported,
                // so we can avoid adding noise to logs when the `opentelemetry` feature is disabled.
                otel.status_code = tracing::field::Empty,
                otel.kind = cfg!(feature = "opentelemetry").then_some("client"),
                error.type = tracing::field::Empty,
                db.system.name = cfg!(feature = "opentelemetry").then_some("clickhouse"),
                // Only log full query text at TRACE level
                // Important that this is taken before client-side parameters are populated
                // FIXME: we can't use `enabled!` due to https://github.com/tokio-rs/tracing/issues/2448
                // but we don't want to log the full query at all verbosity levels.
                // db.query.text = tracing::enabled!(tracing::Level::TRACE).then_some(&sql),
                // TODO: generate summary
                db.query.summary = tracing::field::Empty,
                db.operation.name = "INSERT",
                db.collection.name = collection_name,
                // ClickHouse-specific extension fields
                clickhouse.request.session_id = client.get_setting(settings::SESSION_ID),
                clickhouse.request.query_id = client.get_setting(settings::QUERY_ID),
                clickhouse.request.sent_rows = tracing::field::Empty,
                clickhouse.request.sent_bytes = tracing::field::Empty,
                clickhouse.request.encoded_bytes = tracing::field::Empty,
            ),
            state: InsertState::NotStarted {
                client: Box::new(client.clone()),
                sql,
            },
            #[cfg(any(feature = "lz4", feature = "zstd"))]
            compression: client.compression,
            send_timeout: None,
            end_timeout: None,
            sleep: None,
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
    /// Overrides any roles previously set by this method, [`InsertFormatted::with_setting`],
    /// [`Client::with_roles`] or [`Client::with_setting`].
    ///
    /// An empty iterator may be passed to clear the set roles.
    ///
    /// [roles]: https://clickhouse.com/docs/operations/access-rights#role-management
    ///
    /// # Panics
    /// If called after the request is started, e.g., after [`InsertFormatted::send`].
    pub fn with_roles(mut self, roles: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.state.expect_client_mut().set_roles(roles);
        self
    }

    /// Clear any explicit [roles] previously set on this `Insert` or inherited from [`Client`].
    ///
    /// Overrides any roles previously set by [`InsertFormatted::with_roles`], [`InsertFormatted::with_setting`],
    /// [`Client::with_roles`] or [`Client::with_setting`].
    ///
    /// [roles]: https://clickhouse.com/docs/operations/access-rights#role-management
    ///
    /// # Panics
    /// If called after the request is started, e.g., after [`InsertFormatted::send`].
    pub fn with_default_roles(mut self) -> Self {
        self.state.expect_client_mut().clear_roles();
        self
    }

    /// Similar to [`Client::with_option`], but for this particular INSERT
    /// statement only.
    ///
    /// # Panics
    /// If called after the request is started, e.g., after [`InsertFormatted::send`].
    #[track_caller]
    #[deprecated(since = "0.14.3", note = "please use `with_setting` instead")]
    pub fn with_option(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.state.expect_client_mut().set_setting(name, value);
        self
    }

    /// Similar to [`Client::with_setting`], but for this particular INSERT
    /// statement only.
    ///
    /// # Panics
    /// If called after the request is started, e.g., after [`InsertFormatted::send`].
    #[track_caller]
    pub fn with_setting(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.state.expect_client_mut().set_setting(name, value);
        self
    }

    /// Similar to [`Client::with_product_info()`], but for this `INSERT` statement only.
    ///
    /// # Panics
    /// If called after the request is started, e.g., after [`InsertFormatted::send`].
    pub fn with_product_info(
        mut self,
        product_name: impl Into<String>,
        product_version: impl Into<String>,
    ) -> Self {
        self.state
            .expect_client_mut()
            .add_product_info(product_name.into(), product_version.into());
        self
    }

    pub(crate) fn expect_client(&self) -> &Client {
        self.state.expect_client()
    }

    pub(crate) fn expect_client_mut(&mut self) -> &mut Client {
        self.state.expect_client_mut()
    }

    pub(crate) fn set_timeouts(
        &mut self,
        send_timeout: Option<Duration>,
        end_timeout: Option<Duration>,
    ) {
        self.send_timeout = Timeout::new_opt(send_timeout);
        self.end_timeout = Timeout::new_opt(end_timeout);
    }

    pub(crate) fn span(&self) -> &tracing::Span {
        &self.span
    }

    /// Wrap this `InsertFormatted` with a buffer of a default size.
    ///
    /// The returned type also implements [`AsyncWrite`].
    ///
    /// To set the capacity, use [`Self::buffered_with_capacity()`].
    pub fn buffered(self) -> BufInsertFormatted {
        self.buffered_with_capacity(BUFFER_SIZE)
    }

    /// Wrap this `InsertFormatted` with a buffer of a given size.
    ///
    /// The returned type also implements [`AsyncWrite`].
    ///
    /// If `capacity == 0`, the buffer is flushed between every write regardless of size.
    pub fn buffered_with_capacity(self, capacity: usize) -> BufInsertFormatted {
        BufInsertFormatted::new(self, capacity)
    }

    /// Send a chunk of data.
    ///
    /// If compression is enabled, the data is compressed first.
    ///
    /// To pre-compress the data, use [`Self::send_compressed()`] instead.
    ///
    /// # Note: Unbuffered
    /// This immediately compresses and queues the data to be sent on the connection
    /// without waiting for more chunks. For best performance, chunks should not be too small.
    ///
    /// Use [`Self::buffered()`] for a buffered implementation which also implements [`AsyncWrite`].
    pub async fn send(&mut self, data: Bytes) -> Result<()> {
        let original_size = to_u64_saturating(data.len());

        #[cfg(any(feature = "lz4", feature = "zstd"))]
        let data = if self.compression.is_enabled() {
            CompressedData::new(&data, self.compression)?.compressed
        } else {
            data
        };

        self.send_inner(data, original_size).await
    }

    async fn send_inner(&mut self, mut data: Bytes, original_size: u64) -> Result<()> {
        if self.state.is_not_started() {
            self.init_request()?;
        }

        std::future::poll_fn(move |cx| {
            loop {
                // Potentially cheaper than cloning `data` which touches the refcount
                match self.try_send(mem::take(&mut data), original_size) {
                    ControlFlow::Break(Ok(())) => return Poll::Ready(Ok(())),
                    ControlFlow::Break(Err(_)) => {
                        // If the channel is closed, we should return the actual error
                        return self.poll_wait_handle(cx);
                    }
                    ControlFlow::Continue(unsent) => {
                        data = unsent;
                        // Shorter code-path if we just try to send the data first
                        ready!(self.poll_ready(cx))?;
                    }
                }
            }
        })
        .await
    }

    #[inline]
    pub(crate) fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<()>> {
        if self.state.is_not_started() {
            self.init_request()?;
        }

        let Some(sender) = self.state.sender() else {
            return Poll::Ready(Err(Error::Network("channel closed".into())));
        };

        match sender.poll_ready(cx) {
            Poll::Ready(true) => {
                Timeout::reset_opt(self.send_timeout.as_mut());
                Poll::Ready(Ok(()))
            }
            Poll::Ready(false) => Poll::Ready(Err(Error::Network("channel closed".into()))),
            Poll::Pending => {
                ready!(Timeout::poll_opt(
                    self.send_timeout.as_mut(),
                    &mut self.sleep,
                    cx
                ));
                self.abort();
                Poll::Ready(Err(Error::TimedOut))
            }
        }
    }

    #[inline(always)]
    pub(crate) fn try_send(
        &mut self,
        bytes: Bytes,
        original_size: u64,
    ) -> ControlFlow<Result<()>, Bytes> {
        let InsertState::Active {
            sender,
            sent_bytes,
            encoded_bytes,
            ..
        } = &mut self.state
        else {
            return ControlFlow::Break(Err(Error::Network("channel closed".into())));
        };

        let send_size = bytes.len();

        sender.try_send(bytes).map_break(|res| match res {
            Ok(()) => {
                *sent_bytes += to_u64_saturating(send_size);
                *encoded_bytes += original_size;
                Ok(())
            }
            Err(e) => Err(Error::Network(e.into())),
        })
    }

    /// Ends `INSERT`, the server starts processing the data.
    ///
    /// Succeeds if the server returns 200, that means the `INSERT` was handled
    /// successfully, including all materialized views and quorum writes.
    ///
    /// NOTE: If this isn't called, the whole `INSERT` is aborted.
    pub async fn end(mut self) -> Result<()> {
        std::future::poll_fn(|cx| self.poll_end(cx)).await
    }

    pub(crate) fn poll_end(&mut self, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.state.terminated(&self.span);
        self.poll_wait_handle(cx)
    }

    fn poll_wait_handle(&mut self, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let Some(handle) = self.state.handle() else {
            return Poll::Ready(Ok(()));
        };

        let Poll::Ready(res) = Pin::new(&mut *handle).poll(cx) else {
            ready!(Timeout::poll_opt(
                self.end_timeout.as_mut(),
                &mut self.sleep,
                cx
            ));

            // We can do nothing useful here, so just shut down the background task.
            handle.abort();
            tracing::debug!("insert timed out");
            return Poll::Ready(Err(Error::TimedOut));
        };

        let res = match res {
            Ok(res) => res,
            Err(err) if err.is_panic() => panic::resume_unwind(err.into_panic()),
            Err(err) => Err(Error::Custom(format!("unexpected error: {err}"))),
        };

        self.state = InsertState::Completed;

        tracing::trace!("finished insert");

        Poll::Ready(res.inspect_err(|e| e.record_in_current_span("error from insert query")))
    }

    #[cold]
    #[track_caller]
    #[inline(never)]
    fn init_request(&mut self) -> Result<()> {
        debug_assert!(matches!(self.state, InsertState::NotStarted { .. }));
        let (client, sql) = self.state.client_with_sql().unwrap(); // checked above

        let _span = self.span.enter();

        tracing::trace!("beginning insert");

        let mut url = Url::parse(&client.url).map_err(|err| Error::InvalidParams(err.into()))?;
        let mut pairs = url.query_pairs_mut();
        pairs.clear();

        if let Some(database) = &client.database {
            pairs.append_pair(settings::DATABASE, database);
        }

        pairs.append_pair(settings::QUERY, sql);

        if client.compression.is_enabled() {
            pairs.append_pair(settings::DECOMPRESS, "1");
        }

        for (name, value) in &client.settings {
            pairs.append_pair(name, value);
        }

        drop(pairs);

        let mut builder = Request::post(url.as_str());
        builder = with_request_headers(builder, &client.headers, &client.products_info);
        builder = with_authentication(builder, &client.authentication);

        let (sender, body) = RequestBody::chunked();

        let request = builder.body(body).map_err(|err| {
            let err = Error::InvalidParams(Box::new(err));
            err.record_in_current_span("invalid params in insert request");
            err
        })?;

        let future = client.http.request(request);

        // Ensure the span created internally is captured as a child of the current span.
        let mut response = Response::new(future, Compression::None);

        // TODO: introduce `Executor` to allow bookkeeping of spawned tasks.
        let handle = tokio::spawn(async move { response.finish().await });

        self.state = InsertState::Active {
            handle,
            sender,
            sent_bytes: 0,
            encoded_bytes: 0,
        };
        Ok(())
    }

    pub(crate) fn abort(&mut self) {
        let _span = self.span.enter();

        if let InsertState::Active { sender, .. } = &mut self.state {
            sender.abort();
        }

        self.state.terminated(&self.span);
    }
}

impl Drop for InsertFormatted {
    fn drop(&mut self) {
        self.abort();
    }
}

/// A wrapper around [`InsertFormatted`] which buffers writes.
pub struct BufInsertFormatted {
    insert: InsertFormatted,
    buffer: BytesMut,
    /// Nominal capacity, stored separately because [`Self::write_buffered()`] can grow the buffer.
    nominal_capacity: usize,
}

impl BufInsertFormatted {
    fn new(insert: InsertFormatted, capacity: usize) -> Self {
        Self {
            insert,
            buffer: BytesMut::with_capacity(capacity),
            nominal_capacity: capacity,
        }
    }

    /// Return the number of buffered bytes.
    #[inline(always)]
    pub fn buf_len(&self) -> usize {
        self.buffer.len()
    }

    /// Return the current capacity of the buffer.
    ///
    /// Note: Size is Not Constant
    /// This may be smaller than the original capacity if part of the buffer
    /// is still being used by the connection.
    ///
    /// This may be larger if a call to [`Self::write_buffered()`] caused the buffer to expand.
    #[inline(always)]
    pub fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    /// Return the set capacity of the buffer.
    ///
    /// The current capacity may be larger than this if a call to [`Self::write_buffered()`]
    /// caused the buffer to expand.
    #[inline(always)]
    pub fn nominal_capacity(&self) -> usize {
        self.nominal_capacity
    }

    #[inline(always)]
    pub(crate) fn buffer_mut(&mut self) -> &mut BytesMut {
        &mut self.buffer
    }

    pub(crate) fn expect_client_mut(&mut self) -> &mut Client {
        self.insert.state.expect_client_mut()
    }

    pub(crate) fn set_timeouts(
        &mut self,
        send_timeout: Option<Duration>,
        end_timeout: Option<Duration>,
    ) {
        self.insert.set_timeouts(send_timeout, end_timeout);
    }

    #[doc(hidden)]
    pub fn _priv_span(&self) -> &tracing::Span {
        self.insert.span()
    }

    /// Write data to the buffer without waiting for it to be flushed.
    ///
    /// May cause the buffer to resize to fit the data.
    #[inline(always)]
    pub fn write_buffered(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    /// Write some data to the buffer, flushing first if it is already full.
    ///
    /// Returns the number of bytes written, which may be less than `data.len()` if the remaining
    /// capacity was smaller.
    ///
    /// Cancel-safe. Until this returns `Ok(n)`, the contents of `data` are not yet written to the
    /// buffer.
    // `#[inline]` is *supposed* to work on `async fn`
    // https://doc.rust-lang.org/reference/attributes/codegen.html#r-attributes.codegen.inline.async
    // but it's apparently not implemented yet: https://github.com/rust-lang/rust/pull/149245
    #[inline(always)]
    pub async fn write(&mut self, data: &[u8]) -> Result<usize> {
        std::future::poll_fn(|cx| self.poll_write_inner(data, cx)).await
    }

    // `poll_write` but it returns `crate::Result` instead of `io::Result`
    #[inline(always)]
    fn poll_write_inner(&mut self, data: &[u8], cx: &mut Context<'_>) -> Poll<Result<usize>> {
        // We don't want to wait for the buffer to be full before we start the request,
        // in the event of an error.
        self.init_request_if_required()?;

        // Capacity calculations change a little bit from those in, e.g., `tokio::io::BufWriter`
        // since we always need to copy into the buffer to send chunks on the connection.
        if self.buffer.len() >= self.nominal_capacity {
            ready!(self.poll_flush_inner(cx))?;
            debug_assert!(self.buffer.is_empty());
        }

        // Eliminates the need for a special check in `write_all()`;
        // we need to copy to *some* buffer anyway because of how this type works.
        if self.nominal_capacity == 0 {
            self.buffer.extend_from_slice(data);
            return Poll::Ready(Ok(data.len()));
        }

        // Guaranteed to be >= 1 by the above checks.
        let remaining_capacity = self.nominal_capacity - self.buffer.len();

        let write_len = cmp::min(remaining_capacity, data.len());

        self.buffer.extend_from_slice(&data[..write_len]);
        Poll::Ready(Ok(write_len))
    }

    /// Flush the buffer to the server as a single chunk.
    ///
    /// If [compression is enabled][Client::with_compression], the full buffer will be compressed.
    #[inline(always)]
    pub async fn flush(&mut self) -> Result<()> {
        std::future::poll_fn(|cx| self.poll_flush_inner(cx)).await
    }

    #[inline(always)]
    fn poll_flush_inner(&mut self, cx: &mut Context<'_>) -> Poll<Result<()>> {
        if self.buffer.is_empty() {
            return Poll::Ready(Ok(()));
        }

        ready!(self.insert.poll_ready(cx))?;

        let data = self.buffer.split().freeze();

        let original_size: u64 = data.len().try_into().unwrap_or(u64::MAX);

        #[cfg(any(feature = "lz4", feature = "zstd"))]
        let data = if self.insert.compression.is_enabled() {
            CompressedData::new(&data, self.insert.compression)?.compressed
        } else {
            data
        };

        let ControlFlow::Break(res) = self.insert.try_send(data, original_size) else {
            unreachable!("BUG: we just checked that `ChunkSender` was ready")
        };

        Poll::Ready(res)
    }

    /// Flushes the buffer, then calls [`InsertFormatted::end()`].
    ///
    /// Cancel-safe.
    #[inline(always)]
    pub async fn end(&mut self) -> Result<()> {
        std::future::poll_fn(|cx| self.poll_end(cx)).await
    }

    #[inline(always)]
    fn poll_end(&mut self, cx: &mut Context<'_>) -> Poll<Result<()>> {
        if !self.buffer.is_empty() {
            ready!(self.poll_flush_inner(cx))?;
            debug_assert!(self.buffer.is_empty());
        }

        self.insert.poll_end(cx)
    }

    /// Returns `Ok(true)` if the request was freshly started, `Err(...)` on error,
    /// or `Ok(false)` otherwise.
    #[inline]
    pub(crate) fn init_request_if_required(&mut self) -> Result<bool> {
        if self.insert.state.is_not_started() {
            self.insert.init_request().map(|_| true)
        } else {
            Ok(false)
        }
    }

    pub(crate) fn abort(&mut self) {
        self.insert.abort();
    }
}

impl AsyncWrite for BufInsertFormatted {
    #[inline(always)]
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::result::Result<usize, io::Error>> {
        self.poll_write_inner(buf, cx).map_err(Into::into)
    }

    #[inline(always)]
    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::result::Result<(), io::Error>> {
        self.poll_flush_inner(cx).map_err(Into::into)
    }

    #[inline(always)]
    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::result::Result<(), io::Error>> {
        self.poll_end(cx).map_err(Into::into)
    }
}

impl Timeout {
    fn new_opt(duration: Option<Duration>) -> Option<Self> {
        duration.map(|duration| Self {
            duration,
            is_set: false,
        })
    }

    /// Returns `Poll::Pending` if `None`.
    #[inline(always)]
    fn poll_opt(
        this: Option<&mut Self>,
        sleep: &mut Option<Pin<Box<Sleep>>>,
        cx: &mut Context<'_>,
    ) -> Poll<()> {
        if let Some(this) = this {
            this.poll(sleep, cx)
        } else {
            Poll::Pending
        }
    }

    #[inline]
    fn poll(&mut self, sleep: &mut Option<Pin<Box<Sleep>>>, cx: &mut Context<'_>) -> Poll<()> {
        // Lazily initialize `sleep()` to avoid paying for the allocation if we're not gonna use it.
        // Also avoids the need for a Tokio runtime if just testing setters,
        // e.g. in `insert_native::tests`.
        let sleep = sleep.get_or_insert_with(|| {
            let sleep = Box::pin(tokio::time::sleep(self.duration));
            self.is_set = true;
            sleep
        });

        if !self.is_set {
            let now = Instant::now();

            let deadline = now
                .checked_add(self.duration)
                // What `tokio::time::sleep()` does if the duration overflows `Instant`
                // https://github.com/tokio-rs/tokio/blob/ea91b33ca57ff0581b38e735cc108f831bccbdaa/tokio/src/time/sleep.rs#L126-L129
                // https://github.com/tokio-rs/tokio/blob/ea91b33ca57ff0581b38e735cc108f831bccbdaa/tokio/src/time/instant.rs#L57-L63
                .unwrap_or_else(|| now + Duration::from_secs(86400 * 365 * 30));

            sleep.as_mut().reset(deadline);
            self.is_set = true;
        }

        ready!(sleep.as_mut().poll(cx));
        self.is_set = false;

        Poll::Ready(())
    }

    #[inline(always)]
    fn reset_opt(this: Option<&mut Self>) {
        if let Some(this) = this {
            this.is_set = false;
        }
    }
}

fn to_u64_saturating(n: usize) -> u64 {
    n.try_into().unwrap_or(u64::MAX)
}

// Just so I don't have to repeat this feature flag a hundred times.
#[cfg(any(feature = "lz4", feature = "zstd"))]
mod compression {
    use crate::Compression;
    use crate::error::{Error, Result};
    use crate::insert_formatted::{InsertFormatted, to_u64_saturating};
    use bytes::Bytes;

    /// A chunk of pre-compressed data.
    #[cfg_attr(docsrs, doc(cfg(any(feature = "lz4", feature = "zstd"))))]
    pub struct CompressedData {
        pub(crate) compressed: Bytes,
        pub(crate) original_size: u64,
    }

    impl CompressedData {
        /// Compress a slice of bytes using the specified compression method.
        ///
        /// # Errors
        /// Returns [`Error::Compression`] if `compression` is [`Compression::None`].
        pub fn new(data: &[u8], compression: Compression) -> Result<Self> {
            let original_size = to_u64_saturating(data.len());

            match compression {
                Compression::None => Err(Error::Compression(
                    "cannot pre-compress data when compression is disabled".into(),
                )),
                #[cfg(feature = "lz4")]
                #[allow(deprecated)]
                Compression::Lz4 | Compression::Lz4Hc(_) => Ok(Self {
                    compressed: crate::compression::lz4::compress(data)?,
                    original_size,
                }),
                #[cfg(feature = "zstd")]
                Compression::Zstd(level) => Ok(Self {
                    compressed: crate::compression::zstd::compress(data, Some(level))?,
                    original_size,
                }),
            }
        }

        /// Compress a slice of bytes using LZ4.
        #[cfg(feature = "lz4")]
        #[deprecated(note = "use `CompressedData::new()` instead")]
        #[inline(always)]
        pub fn from_slice(slice: &[u8]) -> Self {
            Self {
                original_size: to_u64_saturating(slice.len()),
                compressed: crate::compression::lz4::compress(slice)
                    .expect("BUG: `lz4::compress()` should not error"),
            }
        }
    }

    #[cfg(feature = "lz4")]
    impl<T> From<T> for CompressedData
    where
        T: AsRef<[u8]>,
    {
        #[inline(always)]
        #[allow(deprecated)]
        fn from(value: T) -> Self {
            Self::from_slice(value.as_ref())
        }
    }

    impl InsertFormatted {
        /// Send a chunk of pre-compressed data.
        ///
        /// # Errors
        /// In addition to network errors, this will return [`Error::Compression`] if the
        /// [`Client`][crate::Client] does not have compression enabled.
        pub async fn send_compressed(&mut self, data: CompressedData) -> Result<()> {
            if !self.compression.is_enabled() {
                return Err(Error::Compression(
                    "attempting to send compressed data, but compression is not enabled".into(),
                ));
            }

            self.send_inner(data.compressed, data.original_size).await
        }
    }
}
