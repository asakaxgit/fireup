// Data importer module for PostgreSQL import functionality
pub mod importer;
pub mod transformer;
pub mod type_mapper;
pub mod sql_generator;

#[cfg(test)]
mod tests;

// Re-export main types and traits
pub use importer::{PostgreSQLImporter, ConnectionConfig, FullImportResult};
pub use transformer::*;
