use super::{ExportConfig, ExportProgressSender, ImportConfig, ImportProgressSender};

pub struct ExportProgressRequest {
    pub connection_id: String,
    pub config: ExportConfig,
    pub progress_tx: Option<ExportProgressSender>,
}

pub struct ImportProgressRequest {
    pub connection_id: String,
    pub config: ImportConfig,
    pub data: String,
    pub file_name: String,
    pub progress_tx: Option<ImportProgressSender>,
}
