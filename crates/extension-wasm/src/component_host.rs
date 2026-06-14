use extension_component::{DbSessionResource, ExtensionDbHost, UiProgressResource};
use wasmtime::component::Resource;

use crate::{
    bindings::onet::extension::{db as Db, task as Task, ui as Ui},
    component::{ComponentCursorResource, ComponentHostState, table_error},
    host_conversions::{
        host_exec_options, host_view_spec, wit_action_context, wit_connection_info, wit_db_error,
        wit_row_batch,
    },
};

impl<H> Db::Host for ComponentHostState<H>
where
    H: ExtensionDbHost + Send + Sync,
{
    async fn list_connections(
        &mut self,
    ) -> wasmtime::Result<Result<Vec<Db::ConnectionInfo>, Db::DbError>> {
        Ok(self
            .db_host
            .list_connections()
            .map(|connections| connections.into_iter().map(wit_connection_info).collect())
            .map_err(wit_db_error))
    }

    async fn open_session(
        &mut self,
        connection_id: String,
        database: Option<String>,
    ) -> wasmtime::Result<Result<Resource<DbSessionResource>, Db::DbError>> {
        let request = extension_component::protocol::OpenSessionRequest {
            connection_id,
            database,
        };
        let session = match self.db_host.open_session(request).await {
            Ok(session) => session,
            Err(error) => return Ok(Err(wit_db_error(error))),
        };
        self.table.push(session).map(Ok).map_err(table_error)
    }
}

impl<H> Db::HostSession for ComponentHostState<H>
where
    H: ExtensionDbHost + Send + Sync,
{
    async fn execute(
        &mut self,
        self_: Resource<DbSessionResource>,
        sql: String,
        options: Db::ExecOptions,
    ) -> wasmtime::Result<Result<Db::RowBatch, Db::DbError>> {
        let session = self.session(&self_)?.clone();
        let options = host_exec_options(options);
        Ok(self
            .db_host
            .execute(&session, sql, options)
            .await
            .map(wit_row_batch)
            .map_err(wit_db_error))
    }

    async fn list_databases(
        &mut self,
        self_: Resource<DbSessionResource>,
    ) -> wasmtime::Result<Result<Vec<String>, Db::DbError>> {
        let session = self.session(&self_)?.clone();
        Ok(self
            .db_host
            .list_databases(&session)
            .await
            .map_err(wit_db_error))
    }

    async fn list_schemas(
        &mut self,
        self_: Resource<DbSessionResource>,
        database: String,
    ) -> wasmtime::Result<Result<Vec<String>, Db::DbError>> {
        let session = self.session(&self_)?.clone();
        Ok(self
            .db_host
            .list_schemas(&session, database)
            .await
            .map_err(wit_db_error))
    }

    async fn close(
        &mut self,
        self_: Resource<DbSessionResource>,
    ) -> wasmtime::Result<Result<(), Db::DbError>> {
        let mut session = self.session(&self_)?.clone();
        let result = self.db_host.close_session(&mut session).await;
        if result.is_ok() {
            self.session_mut(&self_)?.close();
        }
        Ok(result.map_err(wit_db_error))
    }

    async fn drop(&mut self, rep: Resource<DbSessionResource>) -> wasmtime::Result<()> {
        let mut session = self.table.delete(rep).map_err(table_error)?;
        if session.is_closed() {
            return Ok(());
        }
        self.db_host
            .close_session(&mut session)
            .await
            .map_err(|error| wasmtime::Error::msg(error.message))
    }
}

impl<H> Db::HostCursor for ComponentHostState<H>
where
    H: ExtensionDbHost + Send + Sync,
{
    async fn close(
        &mut self,
        self_: Resource<ComponentCursorResource>,
    ) -> wasmtime::Result<Result<(), Db::DbError>> {
        self.table.delete(self_).map_err(table_error)?;
        Ok(Ok(()))
    }

    async fn drop(&mut self, rep: Resource<ComponentCursorResource>) -> wasmtime::Result<()> {
        self.table.delete(rep).map(|_| ()).map_err(table_error)
    }
}

impl<H> Ui::Host for ComponentHostState<H>
where
    H: ExtensionDbHost + Send + Sync,
{
    async fn notify(
        &mut self,
        _level: Ui::NotificationLevel,
        _title: String,
        _message: String,
    ) -> wasmtime::Result<()> {
        Ok(())
    }

    async fn current_action_context(&mut self) -> wasmtime::Result<Option<Ui::ActionContext>> {
        Ok(self.action_context().cloned().map(wit_action_context))
    }

    async fn open_view(&mut self, view: Ui::ViewSpec) -> wasmtime::Result<()> {
        self.push_opened_view(host_view_spec(view));
        Ok(())
    }

    async fn open_result_view(&mut self, _title: String, _payload: String) -> wasmtime::Result<()> {
        Ok(())
    }

    async fn refresh_tree(&mut self, _connection_id: String) -> wasmtime::Result<()> {
        Ok(())
    }

    async fn start_progress(
        &mut self,
        title: String,
    ) -> wasmtime::Result<Resource<UiProgressResource>> {
        let progress = UiProgressResource::new(self.extension_id().to_string(), title);
        self.table.push(progress).map_err(table_error)
    }
}

impl<H> Ui::HostProgress for ComponentHostState<H>
where
    H: ExtensionDbHost + Send + Sync,
{
    async fn set_message(
        &mut self,
        self_: Resource<UiProgressResource>,
        _message: String,
    ) -> wasmtime::Result<()> {
        let _ = self.table.get(&self_).map_err(table_error)?;
        Ok(())
    }

    async fn set_fraction(
        &mut self,
        self_: Resource<UiProgressResource>,
        _fraction: f32,
    ) -> wasmtime::Result<()> {
        let _ = self.table.get(&self_).map_err(table_error)?;
        Ok(())
    }

    async fn close(&mut self, self_: Resource<UiProgressResource>) -> wasmtime::Result<()> {
        self.table.get_mut(&self_).map_err(table_error)?.close();
        Ok(())
    }

    async fn drop(&mut self, rep: Resource<UiProgressResource>) -> wasmtime::Result<()> {
        self.table.delete(rep).map(|_| ()).map_err(table_error)
    }
}

impl<H> Task::Host for ComponentHostState<H>
where
    H: ExtensionDbHost + Send + Sync,
{
    async fn report_status(&mut self, _status: Task::TaskStatus) -> wasmtime::Result<()> {
        Ok(())
    }

    async fn is_cancelled(&mut self, _task_id: String) -> wasmtime::Result<bool> {
        Ok(false)
    }
}
