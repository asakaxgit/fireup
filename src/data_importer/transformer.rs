// Data transformer implementation - placeholder for future tasks
use crate::error::FireupError;
use crate::leveldb_parser::FirestoreDocument;

pub struct TransformationResult {
    pub sql_statements: Vec<String>,
    pub transformed_data: Vec<serde_json::Value>,
    pub warnings: Vec<String>,
}

pub trait DocumentTransformer {
    fn transform_documents(&self, documents: &[FirestoreDocument]) -> Result<TransformationResult, FireupError>;
}