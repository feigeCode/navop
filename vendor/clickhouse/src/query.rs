use hyper::{Method, Request, header::CONTENT_LENGTH};
use serde::Serialize;
use std::fmt::Display;
use tracing::Instrument;
use url::Url;

use crate::{
    Client, Compression,
    error::{Error, Result},
    formats,
    headers::with_request_headers,
    request_body::RequestBody,
    response::Response,
    row::{Row, RowOwned, RowRead},
    sql::{Bind, SqlBuilder, ser},
};

pub use crate::cursors::{BytesCursor, NativeCursor, RowCursor};
use crate::headers::with_authentication;
use crate::settings;
use crate::settings::CLIENT_PROTOCOL_VERSION;

#[must_use]
#[derive(Clone)]
pub struct Query {
    client: Client,
    sql: SqlBuilder,
}

impl Query {
    pub(crate) fn new(client: &Client, template: &str) -> Self {
        Self {
            client: client.clone(),
            sql: SqlBuilder::new(template),
        }
    }

    pub(crate) fn raw(client: &Client, query: &str) -> Self {
        Self {
            client: client.clone(),
            sql: SqlBuilder::raw(query),
        }
    }

    /// Display SQL query as string.
    pub fn sql_display(&self) -> &impl Display {
        &self.sql
    }

    /// Binds `value` to the next `?` in the query.
    ///
    /// The `value`, which must either implement [`Serialize`] or be an
    /// [`Identifier`], will be appropriately escaped.
    ///
    /// All possible errors will be returned as [`Error::InvalidParams`]
    /// during query execution (`execute()`, `fetch()`, etc.).
    ///
    /// WARNING: This means that the query must not have any extra `?`, even if
    /// they are in a string literal! Use `??` to have plain `?` in query.
    ///
    /// [`Serialize`]: serde::Serialize
    /// [`Identifier`]: crate::sql::Identifier
    #[track_caller]
    pub fn bind(mut self, value: impl Bind) -> Self {
        self.sql.bind_arg(value);
        self
    }

    /// Executes the query.
    pub async fn execute(self) -> Result<()> {
        // Enter the span for the `self.do_execute()` call
        let span = self.make_span(None);

        async {
            let mut response = self
                .do_execute(None)
                .inspect_err(|e| e.record_in_current_span("error executing query"))?;

            response
                .finish()
                .await
                .inspect_err(|e| e.record_in_current_span("response error"))
        }
        .instrument(span)
        .await
    }

    /// Executes the query, returning a [`RowCursor`] to obtain results.
    ///
    /// # Example
    ///
    /// ```
    /// # async fn example() -> clickhouse::error::Result<()> {
    /// #[derive(clickhouse::Row, serde::Deserialize)]
    /// struct MyRow<'a> {
    ///     no: u32,
    ///     name: &'a str,
    /// }
    ///
    /// let mut cursor = clickhouse::Client::default()
    ///     .query("SELECT ?fields FROM some WHERE no BETWEEN 0 AND 1")
    ///     .fetch::<MyRow<'_>>()?;
    ///
    /// while let Some(MyRow { name, no }) = cursor.next().await? {
    ///     println!("{name}: {no}");
    /// }
    /// # Ok(()) }
    /// ```
    pub fn fetch<T: Row>(mut self) -> Result<RowCursor<T>> {
        let validation = self.client.get_validation();
        let format = if validation {
            formats::ROW_BINARY_WITH_NAMES_AND_TYPES
        } else {
            formats::ROW_BINARY
        };

        let span = self.make_span(Some(format)).entered();

        self.sql.bind_fields::<T>();

        let response = self
            .do_execute(Some(format))
            .inspect_err(|e| e.record_in_current_span("error executing fetch"))?;

        Ok(RowCursor::new(response, validation, span.exit()))
    }

    /// Executes the query and returns just a single row.
    ///
    /// Note that `T` must be owned.
    pub async fn fetch_one<T>(self) -> Result<T>
    where
        T: RowOwned + RowRead,
    {
        match self.fetch::<T>()?.next().await {
            Ok(Some(row)) => Ok(row),
            Ok(None) => Err(Error::RowNotFound),
            Err(err) => Err(err),
        }
    }

    /// Executes the query and returns at most one row.
    ///
    /// Note that `T` must be owned.
    pub async fn fetch_optional<T>(self) -> Result<Option<T>>
    where
        T: RowOwned + RowRead,
    {
        self.fetch::<T>()?.next().await
    }

    /// Executes the query and returns all the generated results,
    /// collected into a Vec.
    ///
    /// Note that `T` must be owned.
    pub async fn fetch_all<T>(self) -> Result<Vec<T>>
    where
        T: RowOwned + RowRead,
    {
        let mut result = Vec::new();
        let mut cursor = self.fetch::<T>()?;

        while let Some(row) = cursor.next().await? {
            result.push(row);
        }

        Ok(result)
    }

    /// Executes the query, returning a [`BytesCursor`] to obtain results as raw
    /// bytes containing data in the [provided format].
    ///
    /// [provided format]: https://clickhouse.com/docs/en/interfaces/formats
    pub fn fetch_bytes(self, format: impl AsRef<str>) -> Result<BytesCursor> {
        let format = format.as_ref();

        let span = self.make_span(Some(format)).entered();

        let response = self.do_execute(Some(format))?;
        Ok(BytesCursor::new(response, span.exit()))
    }

    pub fn fetch_native(mut self) -> Result<NativeCursor> {
        let span = self.make_span(Some("Native")).entered();

        let client_protocol_version = self.client.settings.get(CLIENT_PROTOCOL_VERSION);

        // Setting `client_protocol_version` to a nonzero value changes the Native response format
        // in ways we are unable to handle, and we would have no way to detect a nonzero version
        // in the response. Return an error if the user is messing around with this setting
        // (which they should not be).
        //
        // https://clickhouse.com/docs/reference/interfaces/specs/NativeFormat#revision-output
        // Original report: https://github.com/ClickHouse/clickhouse-rs/pull/464
        if let Some(version) = client_protocol_version
            && version != "0"
        {
            return Err(Error::Other(format!(
                "Client does not support a nonzero `{CLIENT_PROTOCOL_VERSION}` setting ({version:?}), \
                 please set this setting to \"0\" or leave it unset"
            ).into()));
        }

        // FIXME: use HTTP body compression instead of block-level compression
        self.client = self.client.with_compression(Compression::None);

        let response = self.do_execute(Some("Native"))?;

        Ok(NativeCursor::new(response, span.exit()))
    }

    pub(crate) fn make_span(&self, response_format: Option<&str>) -> tracing::Span {
        // https://opentelemetry.io/docs/specs/semconv/db/sql/
        // TODO: write our own Semantic Conventions for ClickHouse
        tracing::info_span!(
            "clickhouse.query",
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
            // db.query.text = tracing::enabled!(tracing::Level::TRACE).then(|| self.sql.to_string()),
            // TODO: generate summary
            db.query.summary = tracing::field::Empty,
            db.response.status_code = tracing::field::Empty,
            db.response.returned_rows = tracing::field::Empty,
            // ClickHouse-specific extension fields
            clickhouse.request.session_id = self.client.get_setting(settings::SESSION_ID),
            clickhouse.request.query_id = self.client.get_setting(settings::QUERY_ID),
            clickhouse.response.received_bytes = tracing::field::Empty,
            clickhouse.response.decoded_bytes = tracing::field::Empty,
            clickhouse.response.format = response_format,
        )
    }

    pub(crate) fn do_execute(self, default_format: Option<&str>) -> Result<Response> {
        let query = self.sql.finish()?;

        let mut url =
            Url::parse(&self.client.url).map_err(|err| Error::InvalidParams(Box::new(err)))?;
        let mut pairs = url.query_pairs_mut();
        pairs.clear();

        if let Some(format) = default_format {
            pairs.append_pair(settings::DEFAULT_FORMAT, format);
        }

        if let Some(database) = &self.client.database {
            pairs.append_pair(settings::DATABASE, database);
        }

        if self.client.compression.is_enabled() {
            #[cfg(feature = "zstd")]
            if matches!(self.client.compression, crate::Compression::Zstd(_)) {
                pairs.append_pair(settings::ENABLE_HTTP_COMPRESSION, "1");
            } else {
                pairs.append_pair(settings::COMPRESS, "1");
            }

            #[cfg(not(feature = "zstd"))]
            pairs.append_pair(settings::COMPRESS, "1");
        }

        for (name, value) in &self.client.settings {
            pairs.append_pair(name, value);
        }

        pairs.extend_pairs(self.client.roles.iter().map(|role| (settings::ROLE, role)));

        drop(pairs);

        let mut builder = Request::builder().method(Method::POST).uri(url.as_str());
        builder = with_request_headers(builder, &self.client.headers, &self.client.products_info);
        builder = with_authentication(builder, &self.client.authentication);

        #[cfg(feature = "zstd")]
        if matches!(self.client.compression, crate::Compression::Zstd(_)) {
            builder = builder.header("Accept-Encoding", "zstd");
        }

        let content_length = query.len();
        builder = builder.header(CONTENT_LENGTH, content_length.to_string());

        let request = builder.body(RequestBody::full(query)).map_err(|err| {
            let err = Error::InvalidParams(Box::new(err));
            err.record_in_current_span("invalid params in query");
            err
        })?;

        let future = self.client.http.request(request);
        Ok(Response::new(future, self.client.compression))
    }

    /// Configure the [roles] to use when executing this query.
    ///
    /// Overrides any roles previously set by this method, [`Query::with_setting`],
    /// [`Client::with_roles`] or [`Client::with_setting`].
    ///
    /// An empty iterator may be passed to clear the set roles.
    ///
    /// [roles]: https://clickhouse.com/docs/operations/access-rights#role-management
    pub fn with_roles(self, roles: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            client: self.client.with_roles(roles),
            ..self
        }
    }

    /// Clear any explicit [roles] previously set on this `Query` or inherited from [`Client`].
    ///
    /// Overrides any roles previously set by [`Query::with_roles`], [`Query::with_setting`],
    /// [`Client::with_roles`] or [`Client::with_setting`].
    ///
    /// [roles]: https://clickhouse.com/docs/operations/access-rights#role-management
    pub fn with_default_roles(self) -> Self {
        Self {
            client: self.client.with_default_roles(),
            ..self
        }
    }

    /// Similar to [`Client::with_option`], but for this particular query only.
    #[deprecated(since = "0.14.3", note = "please use `with_setting` instead")]
    pub fn with_option(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.client.set_setting(name, value);
        self
    }

    /// Similar to [`Client::with_setting`], but for this particular query only.
    pub fn with_setting(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.client.set_setting(name, value);
        self
    }

    // Used in `clickhouse-ext-arrow` to track Arrow adoption.
    /// Similar to [`Client::with_product_info()`], but for this query only.
    pub fn with_product_info(
        self,
        product_name: impl Into<String>,
        product_version: impl Into<String>,
    ) -> Self {
        Self {
            client: self.client.with_product_info(product_name, product_version),
            ..self
        }
    }

    /// Specify server side parameter for query.
    ///
    /// In queries, you can reference params as {name: type} e.g. {val: Int32}.
    pub fn param(mut self, name: &str, value: impl Serialize) -> Self {
        let mut param = String::from("");
        if let Err(err) = ser::write_param(&mut param, &value) {
            self.sql = SqlBuilder::Failed(format!("invalid param: {err}"));
            self
        } else {
            self.with_setting(format!("param_{name}"), param)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::error::Error;
    use crate::{Client, settings};

    #[test]
    fn fetch_native_errors_nonzero_client_protocol_version() {
        let client = Client::default().with_url("http://localhost:8123");

        // Assert that it does not error by default.
        let _cursor = client.query("SELECT * FROM foo").fetch_native().unwrap();

        let err = client
            .query("SELECT * FROM foo")
            .with_setting(settings::CLIENT_PROTOCOL_VERSION, "54492")
            .fetch_native()
            .err()
            .unwrap_or_else(|| panic!("expected error"));

        let Error::Other(e) = err else {
            panic!("unexpected error kind: {err:?}");
        };

        let err_str = e.to_string();

        assert!(
            err_str.contains("client_protocol_version"),
            "unexpected error: {err_str:?}"
        );
    }
}
