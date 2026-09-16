//! Support for executing an `INSERT` statement using ClickHouse's [Native columnar format].
//!
//! [Native columnar format]: https://clickhouse.com/docs/reference/formats/Native
use crate::insert_formatted::InsertFormatted;
use crate::native::Block;
use crate::native::writer::BlockWriter;
use crate::{Client, Compression, sql};
use std::time::Duration;

/// Executes an `INSERT ... FORMAT Native` statement.
///
/// The [`InsertNative::end`] must be called to finalize the `INSERT`.
/// Otherwise, the whole `INSERT` will be aborted.
#[must_use]
pub struct InsertNative {
    writer: BlockWriter,
}

impl InsertNative {
    pub(crate) fn new(client: &Client, table_name: &str, escape: bool) -> Self {
        let mut sql = "INSERT INTO ".to_string();

        if escape {
            sql::escape::identifier(table_name, &mut sql).expect("error escaping table name");
        } else {
            sql.push_str(table_name);
        }

        sql.push_str(" FORMAT Native");

        Self {
            writer: BlockWriter::new(InsertFormatted::new(
                // FIXME: use HTTP body compression instead of block-level compression
                &client.clone().with_compression(Compression::None),
                sql,
                Some(table_name),
            )),
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
        self.writer
            .insert_mut()
            .set_timeouts(send_timeout, end_timeout);
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
    /// If called after the request is started, i.e., after [`InsertNative::write`].
    pub fn with_roles(mut self, roles: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.writer.expect_client_mut().set_roles(roles);
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
    /// If called after the request is started, i.e., after [`InsertNative::write`].
    pub fn with_default_roles(mut self) -> Self {
        self.writer.expect_client_mut().clear_roles();
        self
    }

    /// Similar to [`Client::with_setting`], but for this particular INSERT
    /// statement only.
    ///
    /// # Panics
    /// If called after the request is started, i.e., after [`InsertNative::write`].
    #[track_caller]
    pub fn with_setting(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.writer.expect_client_mut().set_setting(name, value);
        self
    }

    /// Similar to [`Client::with_product_info()`], but for this `INSERT` statement only.
    ///
    /// # Panics
    /// If called after the request is started, i.e., after [`InsertNative::write`].
    pub fn with_product_info(
        mut self,
        product_name: impl Into<String>,
        product_version: impl Into<String>,
    ) -> Self {
        self.writer
            .expect_client_mut()
            .add_product_info(product_name.into(), product_version.into());
        self
    }

    /// Send a block of data.
    ///
    /// # NOT Cancel Safe
    /// If this `async` method is canceled (i.e. by dropping the resulting `Future`),
    /// the insert is automatically aborted.
    ///
    /// This is because the block data is not sent in a single write, since that would require
    /// copying it into a separate buffer. There is no way to resynchronize the stream
    /// once a block has been partially sent. Resuming a write would corrupt the stream.
    pub async fn write(&mut self, block: &Block) -> crate::Result<()> {
        self.writer.write(block).await
    }

    /// Finish the current `INSERT` request.
    pub async fn end(self) -> crate::Result<()> {
        self.writer.end().await
    }
}

#[cfg(test)]
mod tests {
    use crate::{Client, ProductInfo};

    // These various setters are already covered functionally in `tests/it/insert_formatted.rs`;
    // we just need to check that they're forwarded correctly for `InsertNative`.
    #[test]
    fn test_with_roles() {
        let client = Client::default();

        let insert = client.insert_native("foo");

        assert!(insert.writer.expect_client().roles.is_empty());

        let insert = insert.with_roles(["bar", "baz"]);

        let roles = &insert.writer.expect_client().roles;

        assert_eq!(roles.len(), 2, "unexpected roles: {roles:?}");
        assert!(roles.contains("bar"), "missing role `bar`: {roles:?}");
        assert!(roles.contains("baz"), "missing role `baz`: {roles:?}");

        let insert = insert.with_default_roles();

        assert!(insert.writer.expect_client().roles.is_empty());
    }

    #[test]
    fn test_with_setting() {
        let client = Client::default();

        let insert = client.insert_native("foo");

        let settings = &insert.writer.expect_client().settings;
        assert!(settings.is_empty(), "unexpected settings: {settings:?}");

        let insert = insert.with_setting("foo", "bar").with_setting("bar", "baz");

        let settings = &insert.writer.expect_client().settings;

        assert_eq!(settings.len(), 2, "unexpected settings: {settings:?}");
        assert_eq!(settings.get("foo"), Some(&"bar".to_string()));
        assert_eq!(settings.get("bar"), Some(&"baz".to_string()));
    }

    #[test]
    fn test_with_product_info() {
        let client = Client::default();

        let insert = client.insert_native("foo");

        let product_info = &insert.writer.expect_client().products_info;
        assert!(
            product_info.is_empty(),
            "unexpected product_info: {product_info:?}"
        );

        let insert = insert
            .with_product_info("foo", "1.0.0")
            .with_product_info("bar", "0.1.0-alpha.1");

        let product_info = &insert.writer.expect_client().products_info;

        assert_eq!(
            *product_info,
            [
                ProductInfo {
                    name: "foo".to_string(),
                    version: "1.0.0".to_string(),
                },
                ProductInfo {
                    name: "bar".to_string(),
                    version: "0.1.0-alpha.1".to_string()
                }
            ]
        );
    }
}
