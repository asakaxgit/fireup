// Data importer module for PostgreSQL import functionality
pub mod importer;
pub mod transformer;
pub mod type_mapper;
pub mod sql_generator;

// Re-export main types and traits
pub use importer::{
    PostgreSQLImporter, ConnectionConfig, BatchConfig, BatchProcessor,
    ImportResult, ImportProgress, TableImportSpec, FullImportResult
};
pub use transformer::*;
pub use type_mapper::*;
pub use sql_generator::*;