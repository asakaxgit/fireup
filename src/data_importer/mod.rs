// Data importer module for PostgreSQL import functionality
pub mod connection;
pub mod importer;
pub mod transformer;
pub mod type_mapper;
pub mod sql_generator;

#[cfg(test)]
mod tests;

// Re-export main types and traits
pub use connection::parse_connection_url;
pub use importer::{
    ConnectionConfig, FullImportResult, ImportResult, PostgreSQLImporter, TableImportSpec,
};
pub use transformer::*;
