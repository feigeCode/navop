use super::DatabasePlugin;
use crate::connection::DbConnection;
use crate::types::{DbNode, DbNodeType};
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;

/// Database object tree construction operations.
#[async_trait]
pub trait DatabaseTreeOps: Send + Sync {
    async fn build_database_tree(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
    ) -> Result<Vec<DbNode>>;
    async fn build_schema_tree(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
    ) -> Result<Vec<DbNode>>;
    async fn build_database_or_schema_children(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
        schema: Option<String>,
    ) -> Result<Vec<DbNode>>;
    async fn load_queries(
        &self,
        node: &DbNode,
        metadata: HashMap<String, String>,
    ) -> Result<DbNode>;
    async fn load_node_children(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
    ) -> Result<Vec<DbNode>>;
    async fn load_schema_folder_children(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
        id: &str,
    ) -> Result<Vec<DbNode>>;
    async fn load_queries_children(&self, node: &DbNode, id: &str) -> Result<Vec<DbNode>>;
    async fn load_table_children(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
        id: &str,
    ) -> Result<Vec<DbNode>>;
    #[allow(clippy::too_many_arguments)]
    fn build_table_subfolder(
        &self,
        node: &DbNode,
        parent_id: &str,
        folder_suffix: &str,
        display_prefix: &str,
        folder_type: DbNodeType,
        folder_metadata: &HashMap<String, String>,
        items: Vec<(String, DbNodeType, HashMap<String, String>)>,
    ) -> DbNode;
    async fn load_table_folder_children(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
        id: &str,
    ) -> Result<Vec<DbNode>>;
}

#[async_trait]
impl<T> DatabaseTreeOps for T
where
    T: DatabasePlugin + ?Sized,
{
    async fn build_database_tree(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
    ) -> Result<Vec<DbNode>> {
        DatabasePlugin::build_database_tree(self, connection, node).await
    }

    async fn build_schema_tree(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
    ) -> Result<Vec<DbNode>> {
        DatabasePlugin::build_schema_tree(self, connection, node).await
    }

    async fn build_database_or_schema_children(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
        schema: Option<String>,
    ) -> Result<Vec<DbNode>> {
        DatabasePlugin::build_database_or_schema_children(self, connection, node, schema).await
    }

    async fn load_queries(
        &self,
        node: &DbNode,
        metadata: HashMap<String, String>,
    ) -> Result<DbNode> {
        DatabasePlugin::load_queries(self, node, metadata).await
    }

    async fn load_node_children(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
    ) -> Result<Vec<DbNode>> {
        DatabasePlugin::load_node_children(self, connection, node).await
    }

    async fn load_schema_folder_children(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
        id: &str,
    ) -> Result<Vec<DbNode>> {
        DatabasePlugin::load_schema_folder_children(self, connection, node, id).await
    }

    async fn load_queries_children(&self, node: &DbNode, id: &str) -> Result<Vec<DbNode>> {
        DatabasePlugin::load_queries_children(self, node, id).await
    }

    async fn load_table_children(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
        id: &str,
    ) -> Result<Vec<DbNode>> {
        DatabasePlugin::load_table_children(self, connection, node, id).await
    }

    #[allow(clippy::too_many_arguments)]
    fn build_table_subfolder(
        &self,
        node: &DbNode,
        parent_id: &str,
        folder_suffix: &str,
        display_prefix: &str,
        folder_type: DbNodeType,
        folder_metadata: &HashMap<String, String>,
        items: Vec<(String, DbNodeType, HashMap<String, String>)>,
    ) -> DbNode {
        DatabasePlugin::build_table_subfolder(
            self,
            node,
            parent_id,
            folder_suffix,
            display_prefix,
            folder_type,
            folder_metadata,
            items,
        )
    }

    async fn load_table_folder_children(
        &self,
        connection: &dyn DbConnection,
        node: &DbNode,
        id: &str,
    ) -> Result<Vec<DbNode>> {
        DatabasePlugin::load_table_folder_children(self, connection, node, id).await
    }
}
