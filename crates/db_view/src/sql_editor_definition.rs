//! Cmd/Ctrl+click "object details" provider (IDEA-style).
//!
//! gpui-kit's editor already implements the IDEA interaction: while the
//! secondary modifier (Cmd on macOS, Ctrl elsewhere) is held, it asks the
//! installed `DefinitionProvider` about the identifier under the pointer and
//! underlines a hit; the following Cmd/Ctrl+click runs go-to-definition, which
//! first offers the target to the `show_document` handler. We reuse that
//! plumbing instead of synthesising our own mouse handling: the provider
//! resolves the identifier against the local schema snapshot and hands the
//! details markdown to the handler through `pending`, keyed by a synthetic URI
//! that can never escape the app.

use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use gpui::{App, Task, Window};
use gpui_component::Rope;
use gpui_component::input::DefinitionProvider;
use lsp_types::{LocationLink, Range as LspRange, Uri};

use crate::sql_editor_hover::{
    SqlHoverSources, SqlObjectDetails, offset_to_lsp_position, resolve_object_details,
};
use crate::table_ddl::TableDdlSources;

/// URI scheme marking a click that should open the object-details window.
/// Deliberately not http(s): the handler matches on it, and anything that
/// bypasses the handler would be handed to the platform as an unresolvable
/// scheme instead of navigating a browser.
pub const SQL_OBJECT_DETAILS_SCHEME: &str = "navop-sql-object";

/// Synthetic target used as the `LocationLink` destination; the payload lives
/// in [`DefaultSqlDefinitionProvider::pending`].
const SQL_OBJECT_DETAILS_URI: &str = "navop-sql-object://details";

/// Whether `uri` is our synthetic object-details target.
pub fn is_object_details_uri(uri: &Uri) -> bool {
    uri.scheme().map(|scheme| scheme.as_str()) == Some(SQL_OBJECT_DETAILS_SCHEME)
}

/// Parses [`SQL_OBJECT_DETAILS_URI`].
fn object_details_uri() -> Uri {
    SQL_OBJECT_DETAILS_URI
        .parse()
        .expect("SQL object details URI literal must stay a valid URI")
}

/// Long-lived default definition provider.
///
/// Shares the hover provider's schema snapshot (so a metadata refresh reaches
/// both) and remembers the last resolved details for the click that follows
/// the Cmd/Ctrl+hover.
#[derive(Clone)]
pub struct DefaultSqlDefinitionProvider {
    sources: Rc<RefCell<SqlHoverSources>>,
    pending: Rc<RefCell<Option<SqlObjectDetails>>>,
}

impl DefaultSqlDefinitionProvider {
    pub(crate) fn new(sources: Rc<RefCell<SqlHoverSources>>) -> Self {
        Self {
            sources,
            pending: Rc::new(RefCell::new(None)),
        }
    }

    /// Resolve `offset` and remember the details for the upcoming click.
    ///
    /// Returns `None` (and drops any stale pending details) when the offset is
    /// not on a known object, so the click keeps its normal meaning.
    fn probe_object_details(&self, text: &str, offset: usize) -> Option<LocationLink> {
        let schema = self.sources.borrow().schema.clone();
        let Some(details) = resolve_object_details(text, offset, &schema) else {
            self.pending.borrow_mut().take();
            return None;
        };

        let range = LspRange::new(
            offset_to_lsp_position(text, details.range.start),
            offset_to_lsp_position(text, details.range.end),
        );
        let link = LocationLink {
            // Underlined span while Cmd/Ctrl is held.
            origin_selection_range: Some(range),
            target_uri: object_details_uri(),
            target_range: range,
            target_selection_range: range,
        };
        *self.pending.borrow_mut() = Some(details);
        Some(link)
    }

    /// Details of the last Cmd/Ctrl+hover hit; taken (and cleared) by the
    /// `show_document` handler so one hover yields exactly one window.
    pub(crate) fn take_pending_details(&self) -> Option<SqlObjectDetails> {
        self.pending.borrow_mut().take()
    }

    /// 建表 DDL 来源：与 hover provider 共用同一份快照，所以在这里读也一样。
    pub(crate) fn table_ddl(&self) -> Option<TableDdlSources> {
        self.sources.borrow().table_ddl.clone()
    }
}

impl DefinitionProvider for DefaultSqlDefinitionProvider {
    fn definitions(
        &self,
        text: &Rope,
        offset: usize,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Task<Result<Vec<LocationLink>>> {
        let link = self.probe_object_details(&text.to_string(), offset);
        Task::ready(Ok(link.into_iter().collect()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sql_editor::{SqlColumnDetail, SqlObjectType, SqlSchema, SqlTableDetail};
    use lsp_types::Position as LspPosition;

    fn sample_schema() -> SqlSchema {
        SqlSchema::default()
            .with_scope(Some("app".into()), Some("public".into()))
            .with_tables(vec![("users".to_string(), "doc".to_string())])
            .with_table_detail(
                "users",
                SqlTableDetail {
                    object_type: SqlObjectType::Table,
                    schema: Some("public".into()),
                    comment: None,
                    engine: None,
                    columns: vec![SqlColumnDetail {
                        name: "id".into(),
                        data_type: "INT".into(),
                        is_nullable: false,
                        is_primary_key: true,
                        default_value: None,
                        comment: None,
                    }],
                },
            )
    }

    fn provider(schema: SqlSchema) -> DefaultSqlDefinitionProvider {
        DefaultSqlDefinitionProvider::new(
            crate::sql_editor_hover::DefaultSqlHoverProvider::new(schema).sources_handle(),
        )
    }

    #[test]
    fn probe_returns_link_and_remembers_details_for_a_known_object() {
        let provider = provider(sample_schema());
        let text = "select * from users";
        let offset = text.len();

        let link = provider
            .probe_object_details(text, offset)
            .expect("table name should resolve");

        assert_eq!(Some(object_details_uri()), Some(link.target_uri.clone()));
        assert!(is_object_details_uri(&link.target_uri));
        // The underlined span is the identifier only, not the whole statement.
        let origin = link.origin_selection_range.expect("origin range");
        assert_eq!(
            LspRange::new(LspPosition::new(0, 14), LspPosition::new(0, 19)),
            origin
        );

        let details = provider
            .take_pending_details()
            .expect("details should be primed for the click");
        assert_eq!(details.range, 14..19);
        assert!(details.markdown.contains("**TABLE**"));
        assert_eq!(
            details.table.map(|table| table.name),
            Some("users".to_string()),
            "点击要记住 DDL 坐标，DDL 本体由驱动异步生成"
        );
    }

    #[test]
    fn pending_details_are_consumed_once() {
        let provider = provider(sample_schema());
        let text = "select * from users";

        provider.probe_object_details(text, text.len());
        assert!(provider.take_pending_details().is_some());
        assert!(
            provider.take_pending_details().is_none(),
            "a second click without a fresh hover must not reopen the window"
        );
    }

    #[test]
    fn unresolvable_offset_yields_no_link_and_clears_pending() {
        let provider = provider(sample_schema());
        let text = "select * from users";
        provider.probe_object_details(text, text.len());

        assert!(provider.probe_object_details(text, 0).is_none());
        assert!(
            provider.take_pending_details().is_none(),
            "stale details must not survive a miss"
        );
    }

    #[test]
    fn object_details_uri_is_internal_and_never_external() {
        let uri = object_details_uri();
        assert!(is_object_details_uri(&uri));
        assert_eq!(
            Some(SQL_OBJECT_DETAILS_SCHEME),
            uri.scheme().map(|s| s.as_str())
        );

        let https: Uri = "https://example.com/table".parse().expect("valid uri");
        assert!(!is_object_details_uri(&https));
        let file: Uri = "file:///tmp/query.sql".parse().expect("valid uri");
        assert!(!is_object_details_uri(&file));
    }
}
