// PostgreSQL data importer implementation - placeholder for future tasks
use crate::error::FireupError;

pub struct ImportResult {
    pub imported_records: usize,
    pub failed_records: usize,
    pub warnings: Vec<String>,
}

pub trait PostgreSQLImporter {
    async fn import_data(&self, postgres_url: &str, data: &[serde_json::Value]) -> Result<ImportResult, FireupError>;
}