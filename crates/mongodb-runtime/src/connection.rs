use async_trait::async_trait;
use bson::{Bson, Document};

use crate::types::*;

#[async_trait]
pub trait MongoConnection: Send + Sync {
    fn config(&self) -> &MongoConnectionConfig;

    async fn connect(&mut self) -> Result<(), MongoError>;

    async fn disconnect(&mut self) -> Result<(), MongoError>;

    async fn ping(&self) -> Result<(), MongoError>;

    fn is_connected(&self) -> bool;

    async fn list_databases(&self) -> Result<Vec<String>, MongoError>;

    async fn list_collections(&self, database_name: &str) -> Result<Vec<String>, MongoError>;

    async fn create_collection(
        &self,
        database_name: &str,
        collection_name: &str,
    ) -> Result<(), MongoError>;

    async fn drop_database(&self, database_name: &str) -> Result<(), MongoError>;

    async fn aggregate_documents(
        &self,
        database_name: &str,
        collection_name: &str,
        pipeline: Vec<Document>,
    ) -> Result<Vec<Document>, MongoError>;

    async fn list_indexes(
        &self,
        database_name: &str,
        collection_name: &str,
    ) -> Result<Vec<Document>, MongoError>;

    async fn create_index(
        &self,
        database_name: &str,
        collection_name: &str,
        keys: Document,
        name: Option<String>,
    ) -> Result<(), MongoError>;

    async fn drop_index(
        &self,
        database_name: &str,
        collection_name: &str,
        name: &str,
    ) -> Result<(), MongoError>;

    async fn get_collection_validation(
        &self,
        database_name: &str,
        collection_name: &str,
    ) -> Result<Option<Document>, MongoError>;

    async fn update_collection_validation(
        &self,
        database_name: &str,
        collection_name: &str,
        validator: Option<Document>,
    ) -> Result<(), MongoError>;

    async fn find_documents(
        &self,
        database_name: &str,
        collection_name: &str,
        filter: Option<Document>,
        options: crate::MongoFindOptions,
    ) -> Result<Vec<Document>, MongoError>;

    async fn count_documents(
        &self,
        database_name: &str,
        collection_name: &str,
        filter: Option<Document>,
    ) -> Result<i64, MongoError>;

    async fn insert_document(
        &self,
        database_name: &str,
        collection_name: &str,
        document: Document,
    ) -> Result<(), MongoError>;

    async fn replace_document(
        &self,
        database_name: &str,
        collection_name: &str,
        id: Bson,
        document: Document,
    ) -> Result<(), MongoError>;

    async fn update_document_fields(
        &self,
        database_name: &str,
        collection_name: &str,
        id: Bson,
        set_fields: Document,
    ) -> Result<(), MongoError>;

    async fn delete_document(
        &self,
        database_name: &str,
        collection_name: &str,
        id: Bson,
    ) -> Result<(), MongoError>;

    async fn explain_find(
        &self,
        database_name: &str,
        collection_name: &str,
        filter: Option<Document>,
        options: crate::MongoFindOptions,
    ) -> Result<Document, MongoError>;
}
