#![doc = include_str!("../README.md")]
#![cfg_attr(docsrs, feature(doc_cfg))]

pub use self::{
    compression::Compression,
    query_summary::QuerySummary,
    row::{Row, RowOwned, RowRead, RowWrite},
};
use self::{error::Result, http_client::HttpClient};
use crate::row_metadata::{AccessType, ColumnDefaultKind, InsertMetadata, RowMetadata};

#[doc = include_str!("row_derive.md")]
pub use clickhouse_macros::Row;
use clickhouse_types::{Column, DataTypeNode};

use crate::error::Error;
use std::collections::HashSet;
use std::{collections::HashMap, fmt::Display, sync::Arc};
use tokio::sync::RwLock;

pub mod error;
pub mod insert;
pub mod insert_formatted;
pub mod insert_native;
#[cfg(feature = "inserter")]
pub mod inserter;
pub mod native;

pub mod query;
pub mod serde;
pub mod sql;
#[cfg(feature = "test-util")]
pub mod test;

pub mod types;

mod bytes_ext;
mod compression;
mod cursors;
mod headers;
mod http_client;
mod query_summary;
mod request_body;
mod response;
mod row;
mod row_metadata;
mod rowbinary;
#[cfg(feature = "inserter")]
mod ticks;

/// A client containing HTTP pool.
///
/// ### Cloning behavior
/// Clones share the same HTTP transport but store their own configurations.
/// Any `with_*` configuration method (e.g., [`Client::with_setting`]) applies
/// only to future clones, because [`Client::clone`] creates a deep copy
/// of the [`Client`] configuration, except the transport.
#[derive(Clone)]
pub struct Client {
    http: Arc<dyn HttpClient>,

    url: String,
    database: Option<String>,
    authentication: Authentication,
    compression: Compression,
    roles: HashSet<String>,
    settings: HashMap<String, String>,
    headers: HashMap<String, String>,
    products_info: Vec<ProductInfo>,
    validation: bool,
    insert_metadata_cache: Arc<InsertMetadataCache>,

    #[cfg(feature = "test-util")]
    mocked: bool,
}

#[derive(Clone, Debug)]
#[cfg_attr(test, derive(PartialEq, Eq))]
struct ProductInfo {
    name: String,
    version: String,
}

impl Display for ProductInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.name, self.version)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Authentication {
    Credentials {
        user: Option<String>,
        password: Option<String>,
    },
    Jwt {
        access_token: String,
    },
}

impl Default for Authentication {
    fn default() -> Self {
        Self::Credentials {
            user: None,
            password: None,
        }
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::with_http_client(http_client::default())
    }
}

/// Manual `Debug` implementation to redact sensitive information and omit internal implementation
/// details from the output.
impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // redact auth
        let authentication_redacted = match &self.authentication {
            Authentication::Credentials { .. } => "credentials",
            Authentication::Jwt { .. } => "jwt",
        };
        // redact user/pass in Url
        let origin = url::Url::parse(&self.url)
            .map(|url| url.origin().ascii_serialization())
            .unwrap_or_else(|_| "<invalid url>".to_owned());
        f.debug_struct("Client")
            .field("url", &origin)
            .field("database", &self.database)
            .field("authentication", &authentication_redacted)
            .field("compression", &self.compression)
            .field("roles", &self.roles)
            .field("settings", &self.settings)
            .field("headers", &self.headers.keys()) // redact values
            .field("products_info", &self.products_info)
            .field("validation", &self.validation)
            .finish_non_exhaustive()
    }
}

/// Cache for [`RowMetadata`] to avoid allocating it for the same struct more than once
/// during the application lifecycle. Key: fully qualified table name (e.g. `database.table`).
#[derive(Default)]
pub(crate) struct InsertMetadataCache(RwLock<HashMap<String, Arc<InsertMetadata>>>);

impl Client {
    /// Creates a new client with a specified underlying HTTP client.
    ///
    /// See `HttpClient` for details.
    pub fn with_http_client(client: impl HttpClient) -> Self {
        Self {
            http: Arc::new(client),
            url: String::new(),
            database: None,
            authentication: Authentication::default(),
            compression: Compression::default(),
            roles: HashSet::new(),
            settings: HashMap::new(),
            headers: HashMap::new(),
            products_info: Vec::default(),
            validation: true,
            insert_metadata_cache: Arc::new(InsertMetadataCache::default()),
            #[cfg(feature = "test-util")]
            mocked: false,
        }
    }

    /// Specifies ClickHouse's url. Should point to HTTP endpoint.
    ///
    /// A default database may be set with the `database` query parameter,
    /// e.g. `http://localhost:8123?database=test`. This is equivalent to calling
    /// [`Client::with_database`]: whichever of the two is configured last wins,
    /// and a URL without the parameter leaves the configured database unchanged.
    ///
    /// Automatically [clears the metadata cache][Self::clear_cached_metadata]
    /// for this instance only.
    ///
    /// # Examples
    /// ```
    /// # use clickhouse::Client;
    /// let client = Client::default().with_url("http://localhost:8123");
    /// ```
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();

        // `with_mock()` didn't exist previously, so to not break existing usages,
        // we need to be able to detect a mocked server using nothing but the URL.
        #[cfg(feature = "test-util")]
        if let Some(url) = test::Mock::mocked_url_to_real(&self.url) {
            self.url = url;
            self.mocked = true;
        }

        if let Some(database) = extract_url_database(&self.url) {
            self.database = Some(database);
        }

        // Assume our cached metadata is invalid.
        self.insert_metadata_cache = Default::default();

        self
    }

    /// Specifies a database name.
    ///
    /// Automatically [clears the metadata cache][Self::clear_cached_metadata]
    /// for this instance only.
    ///
    /// # Examples
    /// ```
    /// # use clickhouse::Client;
    /// let client = Client::default().with_database("test");
    /// ```
    pub fn with_database(mut self, database: impl Into<String>) -> Self {
        self.database = Some(database.into());

        // Assume our cached metadata is invalid.
        self.insert_metadata_cache = Default::default();

        self
    }

    /// Returns the default database, if set with [`Client::with_database`]
    /// or the `database` query parameter of the URL (see [`Client::with_url`]).
    pub fn database(&self) -> Option<&str> {
        self.database.as_deref()
    }

    /// Specifies a user.
    ///
    /// # Panics
    /// If called after [`Client::with_access_token`].
    ///
    /// # Examples
    /// ```
    /// # use clickhouse::Client;
    /// let client = Client::default().with_user("test");
    /// ```
    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        match self.authentication {
            Authentication::Jwt { .. } => {
                panic!("`user` cannot be set together with `access_token`");
            }
            Authentication::Credentials { password, .. } => {
                self.authentication = Authentication::Credentials {
                    user: Some(user.into()),
                    password,
                };
            }
        }
        self
    }

    /// Specifies a password.
    ///
    /// # Panics
    /// If called after [`Client::with_access_token`].
    ///
    /// # Examples
    /// ```
    /// # use clickhouse::Client;
    /// let client = Client::default().with_password("secret");
    /// ```
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        match self.authentication {
            Authentication::Jwt { .. } => {
                panic!("`password` cannot be set together with `access_token`");
            }
            Authentication::Credentials { user, .. } => {
                self.authentication = Authentication::Credentials {
                    user,
                    password: Some(password.into()),
                };
            }
        }
        self
    }

    /// Configure the [roles] to use when executing statements with this `Client` instance.
    ///
    /// Overrides any roles previously set by this method or [`Client::with_setting`].
    ///
    /// Call [`Client::with_default_roles`] to clear any explicitly set roles.
    ///
    /// This setting is copied into cloned clients.
    ///
    /// [roles]: https://clickhouse.com/docs/operations/access-rights#role-management
    ///
    /// # Examples
    ///
    /// ```
    /// # use clickhouse::Client;
    ///
    /// // Single role
    /// let client = Client::default().with_roles(["foo"]);
    ///
    /// // Multiple roles
    /// let client = Client::default().with_roles(["foo", "bar", "baz"]);
    /// ```
    pub fn with_roles(mut self, roles: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.set_roles(roles);
        self
    }

    /// Clear any explicitly set [roles] from this `Client` instance.
    ///
    /// Overrides any roles previously set by [`Client::with_roles`] or [`Client::with_setting`].
    ///
    /// [roles]: https://clickhouse.com/docs/operations/access-rights#role-management
    pub fn with_default_roles(mut self) -> Self {
        self.clear_roles();
        self
    }

    /// A JWT access token to authenticate with ClickHouse.
    /// JWT token authentication is supported in ClickHouse Cloud only.
    /// Should not be called after [`Client::with_user`] or
    /// [`Client::with_password`].
    ///
    /// # Panics
    /// If called after [`Client::with_user`] or [`Client::with_password`].
    ///
    /// # Examples
    /// ```
    /// # use clickhouse::Client;
    /// let client = Client::default().with_access_token("jwt");
    /// ```
    pub fn with_access_token(mut self, access_token: impl Into<String>) -> Self {
        match self.authentication {
            Authentication::Credentials { user, password }
                if user.is_some() || password.is_some() =>
            {
                panic!("`access_token` cannot be set together with `user` or `password`");
            }
            _ => {
                self.authentication = Authentication::Jwt {
                    access_token: access_token.into(),
                }
            }
        }
        self
    }

    /// Specifies a compression mode. See [`Compression`] for details.
    /// By default, `Lz4` is used if the `lz4` feature is enabled.
    ///
    /// # Examples
    /// ```
    /// # use clickhouse::{Client, Compression};
    /// # #[cfg(feature = "lz4")]
    /// let client = Client::default().with_compression(Compression::Lz4);
    /// # #[cfg(feature = "zstd")]
    /// let client = Client::default().with_compression(Compression::zstd());
    /// ```
    pub fn with_compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// Used to specify settings that will be passed to all queries.
    ///
    /// # Example
    /// ```
    /// # use clickhouse::Client;
    /// Client::default().with_option("allow_nondeterministic_mutations", "1");
    /// ```
    #[deprecated(since = "0.14.3", note = "please use `with_setting` instead")]
    pub fn with_option(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.settings.insert(name.into(), value.into());
        self
    }

    /// Used to specify settings that will be passed to all queries.
    ///
    /// # Example
    /// ```
    /// # use clickhouse::Client;
    /// Client::default().with_setting("allow_nondeterministic_mutations", "1");
    /// ```
    pub fn with_setting(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.settings.insert(name.into(), value.into());
        self
    }

    /// Used to specify a header that will be passed to all queries.
    ///
    /// # Example
    /// ```
    /// # use clickhouse::Client;
    /// Client::default().with_header("Cookie", "A=1");
    /// ```
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    /// Specifies the product name and version that will be included
    /// in the default User-Agent header. Multiple products are supported.
    /// This could be useful for the applications built on top of this client.
    ///
    /// # Examples
    ///
    /// Sample default User-Agent header:
    ///
    /// ```plaintext
    /// clickhouse-rs/0.12.2 (lv:rust/1.67.0, os:macos)
    /// ```
    ///
    /// Sample User-Agent with a single product information:
    ///
    /// ```
    /// # use clickhouse::Client;
    /// let client = Client::default().with_product_info("MyDataSource", "v1.0.0");
    /// ```
    ///
    /// ```plaintext
    /// MyDataSource/v1.0.0 clickhouse-rs/0.12.2 (lv:rust/1.67.0, os:macos)
    /// ```
    ///
    /// Sample User-Agent with multiple products information
    /// (NB: the products are added in the reverse order of
    /// [`Client::with_product_info`] calls, which could be useful to add
    /// higher abstraction layers first):
    ///
    /// ```
    /// # use clickhouse::Client;
    /// let client = Client::default()
    ///     .with_product_info("MyDataSource", "v1.0.0")
    ///     .with_product_info("MyApp", "0.0.1");
    /// ```
    ///
    /// ```plaintext
    /// MyApp/0.0.1 MyDataSource/v1.0.0 clickhouse-rs/0.12.2 (lv:rust/1.67.0, os:macos)
    /// ```
    pub fn with_product_info(
        mut self,
        product_name: impl Into<String>,
        product_version: impl Into<String>,
    ) -> Self {
        self.add_product_info(product_name.into(), product_version.into());
        self
    }

    pub(crate) fn add_product_info(&mut self, product_name: String, product_version: String) {
        self.products_info.push(ProductInfo {
            name: product_name,
            version: product_version,
        });
    }

    /// Set a setting on this instance of [`Client`].
    ///
    /// Returns the previous value for the setting, if one was set.
    #[deprecated(since = "0.14.3", note = "please use `set_setting` instead")]
    pub fn set_option(
        &mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Option<String> {
        self.settings.insert(name.into(), value.into())
    }

    /// Set a setting on this instance of [`Client`].
    ///
    /// Returns the previous value for the setting, if one was set.
    pub fn set_setting(
        &mut self,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Option<String> {
        self.settings.insert(name.into(), value.into())
    }

    /// Get a setting that was previously set on this `Client`.
    #[deprecated(since = "0.14.3", note = "please use `get_setting` instead")]
    pub fn get_option(&self, name: impl AsRef<str>) -> Option<&str> {
        self.settings.get(name.as_ref()).map(String::as_str)
    }

    /// Get a setting that was previously set on this `Client`.
    pub fn get_setting(&self, name: impl AsRef<str>) -> Option<&str> {
        self.settings.get(name.as_ref()).map(String::as_str)
    }

    /// Starts a new INSERT statement.
    ///
    /// The table name will be escaped as a single identifier. To pass a fully qualified name,
    /// use [`Client::insert_unescaped()`] instead, or override the database name for this statement
    /// using `client.clone().with_database("<db name>")`.
    ///
    /// # Validation
    ///
    /// If validation is enabled (default), `RowBinaryWithNamesAndTypes` input format is used.
    /// When [`Client::insert`] method is called for this `table` for the first time,
    /// it will fetch the table schema from the server, allowing to validate the serialized rows,
    /// as well as write the names and types of the columns in the request header.
    ///
    /// Fetching the schema will happen only once per `table`,
    /// as the schema is cached by the client internally.
    ///
    /// With disabled validation, the schema is not fetched,
    /// and the rows serialized with `RowBinary` input format.
    ///
    /// # Panics
    ///
    /// If `T` has unnamed fields, e.g. tuples.
    pub async fn insert<T: Row>(&self, table: &str) -> Result<insert::Insert<T>> {
        let mut escaped_table_name = String::new();
        sql::escape::identifier(table, &mut escaped_table_name)
            // In practice this should not error, as writing to a `String` should be infallible.
            .map_err(|e| Error::Other(format!("error escaping table name: {e:?}").into()))?;

        self.insert_unescaped(&escaped_table_name).await
    }

    /// Start a new `INSERT` statement using an unescaped table name.
    ///
    /// See [`Client::insert()`] for details.
    pub async fn insert_unescaped<T: Row>(
        &self,
        raw_table_name: &str,
    ) -> Result<insert::Insert<T>> {
        if self.get_validation() {
            let metadata = self.get_insert_metadata(raw_table_name).await?;
            let row = metadata.to_row::<T>()?;
            return Ok(insert::Insert::new(self, raw_table_name, Some(row)));
        }
        Ok(insert::Insert::new(self, raw_table_name, None))
    }

    /// Creates an inserter to perform multiple INSERT statements.
    #[cfg(feature = "inserter")]
    pub fn inserter<T: Row>(&self, table: &str) -> inserter::Inserter<T> {
        inserter::Inserter::new(self, table)
    }

    /// Start an `INSERT` statement sending pre-formatted data.
    ///
    /// `sql` should be an `INSERT INTO ... FORMAT <format name>` statement.
    /// Any other type of statement may produce incorrect results.
    ///
    /// The statement is not issued until the first call to
    /// [`.send()`][insert_formatted::InsertFormatted::send].
    ///
    /// # Note: Not Validated
    /// Unlike [`Insert`][insert::Insert] and [`Inserter`][inserter::Inserter],
    /// this does not perform any validation on the submitted data.
    ///
    /// Only the use of self-describing formats (e.g. CSV, TabSeparated, JSON) is recommended.
    ///
    /// See the [list of supported formats](https://clickhouse.com/docs/interfaces/formats)
    /// for details.
    pub fn insert_formatted_with(
        &self,
        sql: impl Into<String>,
    ) -> insert_formatted::InsertFormatted {
        // TODO: extract collection name from query
        insert_formatted::InsertFormatted::new(self, sql.into(), None)
    }

    pub fn insert_native(&self, table_name: &str) -> insert_native::InsertNative {
        insert_native::InsertNative::new(self, table_name, true)
    }

    pub fn insert_native_unescaped(&self, raw_table_name: &str) -> insert_native::InsertNative {
        insert_native::InsertNative::new(self, raw_table_name, false)
    }

    /// Starts a new SELECT/DDL query.
    pub fn query(&self, query: &str) -> query::Query {
        query::Query::new(self, query)
    }

    /// Starts a new SELECT/DDL query, sending the provided SQL to the server
    /// verbatim, without parsing client-side bind parameters.
    ///
    /// Unlike [`Client::query()`]:
    /// * `?` is not treated as a bind placeholder and is sent as-is,
    ///   so [`Query::bind()`] must not be used with such queries;
    ///   any call to it will result in an [`error::Error::InvalidParams`]
    ///   during query execution (`execute()`, `fetch()`, etc.).
    /// * `??` is not unescaped to `?`.
    /// * `?fields` is not substituted with the [`Row`] column names.
    ///
    /// To parameterize a raw query, use server-side parameters instead:
    /// reference them as `{name: type}` in the SQL and supply values
    /// via [`Query::param()`].
    ///
    /// # Example
    ///
    /// ```
    /// # async fn example() -> clickhouse::error::Result<()> {
    /// let value: String = clickhouse::Client::default()
    ///     .query_raw("SELECT concat('foo?', {suffix: String})")
    ///     .param("suffix", "bar")
    ///     .fetch_one()
    ///     .await?;
    /// assert_eq!(value, "foo?bar");
    /// # Ok(()) }
    /// ```
    ///
    /// [`Query::bind()`]: query::Query::bind
    /// [`Query::param()`]: query::Query::param
    pub fn query_raw(&self, query: &str) -> query::Query {
        query::Query::raw(self, query)
    }

    /// Enables or disables [`Row`] data types validation against the database schema
    /// at the cost of performance. Validation is enabled by default, and in this mode,
    /// the client will use `RowBinaryWithNamesAndTypes` format.
    ///
    /// If you are looking to maximize performance, you could disable validation using this method.
    /// When validation is disabled, the client switches to `RowBinary` format usage instead.
    ///
    /// The downside with plain `RowBinary` is that instead of clearer error messages,
    /// a mismatch between [`Row`] and database schema will result
    /// in a [`error::Error::NotEnoughData`] error without specific details.
    ///
    /// However, depending on the dataset, there might be x1.1 to x3 performance improvement,
    /// but that highly depends on the shape and volume of the dataset.
    ///
    /// It is always recommended to measure the performance impact of validation
    /// in your specific use case. Additionally, writing smoke tests to ensure that
    /// the row types match the ClickHouse schema is highly recommended,
    /// if you plan to disable validation in your application.
    ///
    /// # Note: Mocking
    /// When using [`test::Mock`] with the `test-util` feature, validation is forced off.
    ///
    /// This applies either when using [`Client::with_mock()`], or [`Client::with_url()`]
    /// with a URL from [`test::Mock::url()`].
    ///
    /// As of writing, the mocking facilities are unable to generate the `RowBinaryWithNamesAndTypes`
    /// header required for validation to function.
    pub fn with_validation(mut self, enabled: bool) -> Self {
        self.validation = enabled;
        self
    }

    /// Clear table metadata that was previously received and cached.
    ///
    /// [`Insert`][crate::insert::Insert] uses cached metadata when sending data with validation.
    /// If the table schema changes, this metadata needs to re-fetched.
    ///
    /// This method clears the metadata cache, causing future insert queries to re-fetch metadata.
    /// This applies to all cloned instances of this `Client` (using the same URL and database)
    /// as well.
    ///
    /// This may need to wait to acquire a lock if a query is concurrently writing into the cache.
    ///
    /// Cancel-safe.
    pub async fn clear_cached_metadata(&self) {
        self.insert_metadata_cache.0.write().await.clear();
    }

    /// Used internally to check if the validation mode is enabled,
    /// as it takes into account the `test-util` feature flag.
    #[inline]
    pub(crate) fn get_validation(&self) -> bool {
        #[cfg(feature = "test-util")]
        if self.mocked {
            return false;
        }
        self.validation
    }

    pub(crate) fn set_roles(&mut self, roles: impl IntoIterator<Item = impl Into<String>>) {
        self.clear_roles();
        self.roles.extend(roles.into_iter().map(Into::into));
    }

    #[inline]
    pub(crate) fn clear_roles(&mut self) {
        // Make sure we overwrite any role manually set by the user via `with_setting()`.
        self.settings.remove(settings::ROLE);
        self.roles.clear();
    }

    /// Use a mock server for testing purposes.
    ///
    /// # Note
    ///
    /// The client will always use `RowBinary` format instead of `RowBinaryWithNamesAndTypes`,
    /// as otherwise it'd be required to provide RBWNAT header in the mocks,
    /// which is pointless in that kind of tests.
    #[cfg(feature = "test-util")]
    pub fn with_mock(mut self, mock: &test::Mock) -> Self {
        self.url = mock.real_url().to_string();
        self.mocked = true;
        self
    }

    async fn get_insert_metadata(&self, raw_table_name: &str) -> Result<Arc<InsertMetadata>> {
        #[derive(::serde::Deserialize, clickhouse_macros::Row)]
        #[clickhouse(crate = "self")]
        // `Row` derive doesn't allow omitting columns
        #[expect(dead_code)]
        struct DescribeColumn {
            name: String,
            r#type: String,
            default_type: String,
            default_expression: String,
            comment: String,
            codec_expression: String,
            ttl_expression: String,
        }

        {
            let read_lock = self.insert_metadata_cache.0.read().await;

            if let Some(metadata) = read_lock.get(raw_table_name) {
                return Ok(metadata.clone());
            }
        }

        // TODO: should it be moved to a cold function?
        let mut write_lock = self.insert_metadata_cache.0.write().await;

        let mut columns_cursor = self
            .query(&_priv::row_insert_metadata_query(raw_table_name))
            .with_setting("describe_include_subcolumns", "0")
            .fetch::<DescribeColumn>()?;

        let mut columns = Vec::new();
        let mut column_default_kinds = Vec::new();
        let mut column_lookup = HashMap::new();

        while let Some(column) = columns_cursor.next().await? {
            let data_type = DataTypeNode::new(&column.r#type)?;
            let default_kind = column.default_type.parse::<ColumnDefaultKind>()?;

            column_lookup.insert(column.name.clone(), columns.len());

            columns.push(Column {
                name: column.name,
                data_type,
            });

            column_default_kinds.push(default_kind);
        }

        let metadata = Arc::new(InsertMetadata {
            row_metadata: RowMetadata {
                columns,
                access_type: AccessType::WithSeqAccess, // ignored on insert
            },
            column_default_kinds,
            column_lookup,
        });

        write_lock.insert(raw_table_name.to_string(), metadata.clone());
        Ok(metadata)
    }
}

/// Extracts the last non-empty `database` query parameter from the URL, if any.
/// Invalid URLs are ignored here; the error surfaces at request time.
fn extract_url_database(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;

    url.query_pairs()
        .filter(|(key, value)| key == settings::DATABASE && !value.is_empty())
        .map(|(_, value)| value.into_owned())
        .last()
}

mod formats {
    pub(crate) const ROW_BINARY: &str = "RowBinary";
    pub(crate) const ROW_BINARY_WITH_NAMES_AND_TYPES: &str = "RowBinaryWithNamesAndTypes";
}

mod settings {
    pub(crate) const DATABASE: &str = "database";
    pub(crate) const DEFAULT_FORMAT: &str = "default_format";
    pub(crate) const COMPRESS: &str = "compress";
    pub(crate) const DECOMPRESS: &str = "decompress";
    #[cfg(feature = "zstd")]
    pub(crate) const ENABLE_HTTP_COMPRESSION: &str = "enable_http_compression";
    pub(crate) const ROLE: &str = "role";
    pub(crate) const QUERY: &str = "query";
    pub(crate) const QUERY_ID: &str = "query_id";
    pub(crate) const SESSION_ID: &str = "session_id";
    pub(crate) const CLIENT_PROTOCOL_VERSION: &str = "client_protocol_version";
}

/// This is a private API exported only for internal purposes.
/// Do not use it in your code directly, it doesn't follow semver.
#[doc(hidden)]
pub mod _priv {
    pub use crate::row::RowKind;

    #[cfg(feature = "lz4")]
    pub fn lz4_compress(uncompressed: &[u8]) -> super::Result<bytes::Bytes> {
        crate::compression::lz4::compress(uncompressed)
    }

    #[cfg(feature = "zstd")]
    pub fn zstd_compress(uncompressed: &[u8]) -> super::Result<bytes::Bytes> {
        crate::compression::zstd::compress(uncompressed, None)
    }

    // Also needed by `it::insert::cache_row_metadata()`
    pub fn row_insert_metadata_query(raw_table: &str) -> String {
        format!("DESCRIBE TABLE {raw_table}")
    }

    pub use crate::sql::escape::identifier as sql_escape_identifier;
}

#[cfg(test)]
mod client_tests {
    use crate::_priv::RowKind;
    use crate::row_metadata::{AccessType, RowMetadata};
    use crate::{Authentication, Client, Compression, Row};
    use clickhouse_types::{Column, DataTypeNode};

    #[test]
    fn it_can_use_credentials_auth() {
        assert_eq!(
            Client::default()
                .with_user("bob")
                .with_password("secret")
                .authentication,
            Authentication::Credentials {
                user: Some("bob".into()),
                password: Some("secret".into()),
            }
        );
    }

    #[test]
    fn it_can_use_credentials_auth_user_only() {
        assert_eq!(
            Client::default().with_user("alice").authentication,
            Authentication::Credentials {
                user: Some("alice".into()),
                password: None,
            }
        );
    }

    #[test]
    fn it_can_use_credentials_auth_password_only() {
        assert_eq!(
            Client::default().with_password("secret").authentication,
            Authentication::Credentials {
                user: None,
                password: Some("secret".into()),
            }
        );
    }

    #[test]
    fn it_can_override_credentials_auth() {
        assert_eq!(
            Client::default()
                .with_user("bob")
                .with_password("secret")
                .with_user("alice")
                .with_password("something_else")
                .authentication,
            Authentication::Credentials {
                user: Some("alice".into()),
                password: Some("something_else".into()),
            }
        );
    }

    #[test]
    fn it_can_use_jwt_auth() {
        assert_eq!(
            Client::default().with_access_token("my_jwt").authentication,
            Authentication::Jwt {
                access_token: "my_jwt".into(),
            }
        );
    }

    #[test]
    fn it_can_override_jwt_auth() {
        assert_eq!(
            Client::default()
                .with_access_token("my_jwt")
                .with_access_token("my_jwt_2")
                .authentication,
            Authentication::Jwt {
                access_token: "my_jwt_2".into(),
            }
        );
    }

    #[test]
    #[should_panic(expected = "`access_token` cannot be set together with `user` or `password`")]
    fn it_cannot_use_jwt_after_with_user() {
        let _ = Client::default()
            .with_user("bob")
            .with_access_token("my_jwt");
    }

    #[test]
    #[should_panic(expected = "`access_token` cannot be set together with `user` or `password`")]
    fn it_cannot_use_jwt_after_with_password() {
        let _ = Client::default()
            .with_password("secret")
            .with_access_token("my_jwt");
    }

    #[test]
    #[should_panic(expected = "`access_token` cannot be set together with `user` or `password`")]
    fn it_cannot_use_jwt_after_both_with_user_and_with_password() {
        let _ = Client::default()
            .with_user("alice")
            .with_password("secret")
            .with_access_token("my_jwt");
    }

    #[test]
    #[should_panic(expected = "`user` cannot be set together with `access_token`")]
    fn it_cannot_use_with_user_after_jwt() {
        let _ = Client::default()
            .with_access_token("my_jwt")
            .with_user("alice");
    }

    #[test]
    #[should_panic(expected = "`password` cannot be set together with `access_token`")]
    fn it_cannot_use_with_password_after_jwt() {
        let _ = Client::default()
            .with_access_token("my_jwt")
            .with_password("secret");
    }

    #[test]
    fn it_sets_validation_mode() {
        let client = Client::default();
        assert!(client.validation);
        let client = client.with_validation(false);
        assert!(!client.validation);
        let client = client.with_validation(true);
        assert!(client.validation);
    }

    #[derive(Debug, Clone, PartialEq)]
    struct SystemRolesRow {
        name: String,
        id: uuid::Uuid,
        storage: String,
    }

    impl SystemRolesRow {
        fn columns() -> Vec<Column> {
            vec![
                Column::new("name".to_string(), DataTypeNode::String),
                Column::new("id".to_string(), DataTypeNode::UUID),
                Column::new("storage".to_string(), DataTypeNode::String),
            ]
        }
    }

    impl Row for SystemRolesRow {
        const NAME: &'static str = "SystemRolesRow";
        const KIND: RowKind = RowKind::Struct;
        const COLUMN_COUNT: usize = 3;
        const COLUMN_NAMES: &'static [&'static str] = &["name", "id", "storage"];
        type Value<'a> = SystemRolesRow;
    }

    #[test]
    fn get_row_metadata() {
        let metadata =
            RowMetadata::new_for_cursor::<SystemRolesRow>(SystemRolesRow::columns()).unwrap();
        assert_eq!(metadata.columns, SystemRolesRow::columns());
        assert_eq!(metadata.access_type, AccessType::WithSeqAccess);

        // the order is shuffled => map access
        let columns = vec![
            Column::new("id".to_string(), DataTypeNode::UUID),
            Column::new("storage".to_string(), DataTypeNode::String),
            Column::new("name".to_string(), DataTypeNode::String),
        ];
        let metadata = RowMetadata::new_for_cursor::<SystemRolesRow>(columns.clone()).unwrap();
        assert_eq!(metadata.columns, columns);
        assert_eq!(
            metadata.access_type,
            AccessType::WithMapAccess(vec![1, 2, 0]) // see COLUMN_NAMES above
        );
    }

    #[test]
    fn it_does_follow_previous_configuration() {
        let client = Client::default().with_setting("async_insert", "1");
        assert_eq!(client.settings, client.clone().settings,);
    }

    #[test]
    fn it_does_not_follow_future_configuration() {
        let client = Client::default();
        let client_clone = client.clone();
        let client = client.with_setting("async_insert", "1");
        assert_ne!(client.settings, client_clone.settings,);
    }

    #[test]
    fn it_recognizes_url_database_param() {
        fn database_of(url: &str) -> Option<String> {
            Client::default().with_url(url).database().map(Into::into)
        }

        assert_eq!(Client::default().database(), None);
        assert_eq!(database_of("http://localhost:8123"), None);
        assert_eq!(
            database_of("http://localhost:8123?database=foo"),
            Some("foo".into())
        );
        // Percent-encoded values are decoded
        assert_eq!(
            database_of("http://localhost:8123?database=weird%20db"),
            Some("weird db".into())
        );
        // Other parameters don't interfere; the last non-empty occurrence wins
        assert_eq!(
            database_of("http://localhost:8123?compress=1&database=foo&database=bar"),
            Some("bar".into())
        );
        // Empty values and invalid URLs are ignored
        assert_eq!(database_of("http://localhost:8123?database="), None);
        assert_eq!(database_of("not a url"), None);

        // The last configured database wins, whether from the URL or `with_database()`
        let client = Client::default()
            .with_url("http://localhost:8123?database=from_url")
            .with_database("explicit");
        assert_eq!(client.database(), Some("explicit"));

        let client = Client::default()
            .with_database("explicit")
            .with_url("http://localhost:8123?database=from_url");
        assert_eq!(client.database(), Some("from_url"));

        // A URL without the parameter leaves the configured database unchanged
        let client = Client::default()
            .with_database("explicit")
            .with_url("http://localhost:8123");
        assert_eq!(client.database(), Some("explicit"));
    }

    #[test]
    fn client_debug() {
        let client = Client::default()
            .with_url("http://localhost:8123")
            .with_database("mydb")
            .with_user("zach")
            .with_password("verysecretpassword")
            .with_compression(Compression::None)
            .with_roles(["reader"])
            .with_setting("async_insert", "1")
            .with_header("X-Trace-Id", "abc")
            .with_product_info("MyApp", "0.0.1")
            .with_validation(false);

        let dbg = format!("{client:#?}");
        let expected = "\
Client {
    url: \"http://localhost:8123\",
    database: Some(
        \"mydb\",
    ),
    authentication: \"credentials\",
    compression: None,
    roles: {
        \"reader\",
    },
    settings: {
        \"async_insert\": \"1\",
    },
    headers: [
        \"X-Trace-Id\",
    ],
    products_info: [
        ProductInfo {
            name: \"MyApp\",
            version: \"0.0.1\",
        },
    ],
    validation: false,
    ..
}";
        assert_eq!(dbg, expected);
    }

    #[test]
    fn client_debug_redaction() {
        let client = Client::default()
            .with_url("http://urluser:urlsecret@localhost:8123")
            .with_database("mydb")
            .with_user("zach")
            .with_password("verysecretpassword")
            .with_header("Authorization", "Bearer super-secret")
            .with_header("X-Trace-Id", "abc");

        let dbg = format!("{client:?}");
        assert!(
            !dbg.contains("verysecretpassword"),
            "password leaked: {dbg}"
        );
        assert!(!dbg.contains("zach"), "user leaked: {dbg}");
        // credentials embedded in the URL are redacted down to the origin
        assert!(!dbg.contains("urluser"), "url user leaked: {dbg}");
        assert!(!dbg.contains("urlsecret"), "url password leaked: {dbg}");
        assert!(!dbg.contains("super-secret"), "header value leaked: {dbg}");
        assert!(dbg.contains("\"credentials\""), "missing auth tag: {dbg}");
        assert!(
            dbg.contains("http://localhost:8123"),
            "missing origin: {dbg}"
        );
        assert!(dbg.contains("mydb"), "missing database: {dbg}");
        // header names visible
        assert!(dbg.contains("Authorization"), "missing header name: {dbg}");
        assert!(dbg.contains("X-Trace-Id"), "missing header name: {dbg}");

        // JWT: access token must not appear
        let client = Client::default().with_access_token("eyJhbGciOi.payload.signature");
        let dbg = format!("{client:?}");
        assert!(!dbg.contains("eyJhbGciOi"), "access token leaked: {dbg}");
        assert!(dbg.contains("\"jwt\""), "missing auth tag: {dbg}");
    }

    #[test]
    fn client_debug_invalid_url() {
        // An empty URL (the default) cannot be parsed; `origin` falls back to a
        // placeholder rather than echoing the raw string.
        let dbg = format!("{:?}", Client::default());
        assert!(dbg.contains("url: \"<invalid url>\""), "unexpected: {dbg}");

        // A malformed URL must not be echoed verbatim even on the fallback path,
        // since it may still contain credentials.
        let dbg = format!(
            "{:?}",
            Client::default().with_url("not a url but has a secret hunter2")
        );
        assert!(dbg.contains("url: \"<invalid url>\""), "unexpected: {dbg}");
        assert!(!dbg.contains("hunter2"), "raw url leaked: {dbg}");
    }

    #[test]
    fn it_gets_and_sets_settings() {
        let mut client = Client::default();

        assert_eq!(client.set_setting("foo", "foo"), None);
        assert_eq!(client.set_setting("bar", "bar"), None);

        assert_eq!(client.get_setting("foo"), Some("foo"));
        assert_eq!(client.get_setting("bar"), Some("bar"));
        assert_eq!(client.get_setting("baz"), None);

        assert_eq!(client.set_setting("foo", "foo_2"), Some("foo".to_string()));
        assert_eq!(client.set_setting("bar", "bar_2"), Some("bar".to_string()));
    }
}
