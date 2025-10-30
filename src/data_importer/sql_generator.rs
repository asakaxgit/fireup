use crate::error::FireupError;
use crate::types::{TableDefinition, PostgreSQLType, NormalizedSchema, PrimaryKeyDefinition};
use crate::data_importer::transformer::{TableRow, TransformationResult};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Configuration for SQL generation
#[derive(Debug, Clone)]
pub struct SQLGenerationConfig {
    /// Maximum number of rows per INSERT statement
    pub batch_size: usize,
    /// Whether to use parameterized queries
    pub use_parameters: bool,
    /// Whether to include ON CONFLICT clauses
    pub handle_conflicts: bool,
    /// Conflict resolution strategy
    pub conflict_strategy: ConflictStrategy,
    /// Whether to wrap statements in transactions
    pub use_transactions: bool,
}

/// Strategy for handling conflicts during INSERT
#[derive(Debug, Clone)]
pub enum ConflictStrategy {
    /// Ignore conflicts (ON CONFLICT DO NOTHING)
    Ignore,
    /// Update on conflict (ON CONFLICT DO UPDATE)
    Update,
    /// Fail on conflict (default PostgreSQL behavior)
    Fail,
}

impl Default for SQLGenerationConfig {
    fn default() -> Self {
        Self {
            batch_size: 1000,
            use_parameters: true,
            handle_conflicts: true,
            conflict_strategy: ConflictStrategy::Update,
            use_transactions: true,
        }
    }
}

/// Result of SQL generation process
#[derive(Debug, Clone)]
pub struct SQLGenerationResult {
    /// Generated SQL statements
    pub statements: Vec<SQLStatement>,
    /// Total number of rows that will be inserted
    pub total_rows: u64,
    /// Warnings encountered during generation
    pub warnings: Vec<String>,
    /// Statistics about the generation
    pub statistics: SQLGenerationStatistics,
}

/// A single SQL statement with metadata
#[derive(Debug, Clone)]
pub struct SQLStatement {
    /// The SQL statement text
    pub sql: String,
    /// Parameters for parameterized queries
    pub parameters: Vec<Value>,
    /// Table this statement affects
    pub table_name: String,
    /// Number of rows this statement will insert
    pub row_count: usize,
    /// Statement type
    pub statement_type: StatementType,
}

/// Type of SQL statement
#[derive(Debug, Clone, PartialEq)]
pub enum StatementType {
    /// CREATE TABLE statement
    CreateTable,
    /// INSERT statement
    Insert,
    /// CREATE INDEX statement
    CreateIndex,
    /// ALTER TABLE statement (for constraints)
    AlterTable,
    /// Transaction control
    Transaction,
}

/// Statistics about SQL generation
#[derive(Debug, Clone)]
pub struct SQLGenerationStatistics {
    /// Number of INSERT statements generated
    pub insert_statements: u32,
    /// Number of CREATE statements generated
    pub create_statements: u32,
    /// Total number of parameters used
    pub total_parameters: u32,
    /// Generation time in milliseconds
    pub generation_time_ms: u64,
}

/// Generates SQL statements for PostgreSQL import
pub struct SQLGenerator {
    /// Configuration for SQL generation
    config: SQLGenerationConfig,
    /// Parameter counter for parameterized queries
    parameter_counter: u32,
}

impl SQLGenerator {
    /// Create a new SQL generator with default configuration
    pub fn new() -> Self {
        Self {
            config: SQLGenerationConfig::default(),
            parameter_counter: 0,
        }
    }

    /// Create a new SQL generator with custom configuration
    pub fn with_config(config: SQLGenerationConfig) -> Self {
        Self {
            config,
            parameter_counter: 0,
        }
    }

    /// Generate SQL statements from transformation result
    pub fn generate_sql(
        &mut self,
        transformation_result: &TransformationResult,
        schema: &NormalizedSchema,
    ) -> Result<SQLGenerationResult, FireupError> {
        let start_time = std::time::Instant::now();
        let mut statements = Vec::new();
        let mut warnings = Vec::new();
        let mut statistics = SQLGenerationStatistics {
            insert_statements: 0,
            create_statements: 0,
            total_parameters: 0,
            generation_time_ms: 0,
        };

        // Generate transaction start if configured
        if self.config.use_transactions {
            statements.push(SQLStatement {
                sql: "BEGIN;".to_string(),
                parameters: vec![],
                table_name: "".to_string(),
                row_count: 0,
                statement_type: StatementType::Transaction,
            });
        }

        // Generate CREATE TABLE statements
        for table in &schema.tables {
            let create_statement = self.generate_create_table_statement(table)?;
            statements.push(create_statement);
            statistics.create_statements += 1;
        }

        // Generate INSERT statements in dependency order
        let insert_statements = self.generate_insert_statements(
            &transformation_result.table_data,
            schema,
            &mut warnings,
        )?;
        
        statistics.insert_statements = insert_statements.len() as u32;
        statements.extend(insert_statements);

        // Generate CREATE INDEX statements
        for table in &schema.tables {
            let index_statements = self.generate_index_statements(table)?;
            statistics.create_statements += index_statements.len() as u32;
            statements.extend(index_statements);
        }

        // Generate ALTER TABLE statements for foreign keys
        let constraint_statements = self.generate_constraint_statements(schema)?;
        statistics.create_statements += constraint_statements.len() as u32;
        statements.extend(constraint_statements);

        // Generate transaction commit if configured
        if self.config.use_transactions {
            statements.push(SQLStatement {
                sql: "COMMIT;".to_string(),
                parameters: vec![],
                table_name: "".to_string(),
                row_count: 0,
                statement_type: StatementType::Transaction,
            });
        }

        statistics.total_parameters = self.parameter_counter;
        statistics.generation_time_ms = start_time.elapsed().as_millis() as u64;

        let total_rows = transformation_result.table_data.values()
            .map(|rows| rows.len() as u64)
            .sum();

        Ok(SQLGenerationResult {
            statements,
            total_rows,
            warnings,
            statistics,
        })
    }

    /// Generate CREATE TABLE statement for a table definition
    pub fn generate_create_table_statement(&self, table: &TableDefinition) -> Result<SQLStatement, FireupError> {
        let mut sql = format!("CREATE TABLE IF NOT EXISTS {} (\n", table.name);
        
        let mut column_definitions = Vec::new();
        
        // Add column definitions
        for column in &table.columns {
            let mut column_def = format!("  {} {}", column.name, column.column_type.to_sql());
            
            if !column.nullable {
                column_def.push_str(" NOT NULL");
            }
            
            if let Some(default_value) = &column.default_value {
                column_def.push_str(&format!(" DEFAULT {}", self.format_sql_value(default_value)));
            }
            
            column_definitions.push(column_def);
        }
        
        // Add primary key constraint
        if let Some(ref pk) = table.primary_key {
            let pk_constraint = format!(
                "  CONSTRAINT {} PRIMARY KEY ({})",
                pk.name,
                pk.columns.join(", ")
            );
            column_definitions.push(pk_constraint);
        }
        
        sql.push_str(&column_definitions.join(",\n"));
        sql.push_str("\n);");
        
        Ok(SQLStatement {
            sql,
            parameters: vec![],
            table_name: table.name.clone(),
            row_count: 0,
            statement_type: StatementType::CreateTable,
        })
    }

    /// Generate INSERT statements for all tables in dependency order
    fn generate_insert_statements(
        &mut self,
        table_data: &HashMap<String, Vec<TableRow>>,
        schema: &NormalizedSchema,
        warnings: &mut Vec<String>,
    ) -> Result<Vec<SQLStatement>, FireupError> {
        let mut statements = Vec::new();
        let mut processed_tables = HashSet::new();
        let mut remaining_tables: Vec<&TableDefinition> = schema.tables.iter().collect();

        // Process tables in dependency order
        while !remaining_tables.is_empty() {
            let mut progress_made = false;

            remaining_tables.retain(|table| {
                // Check if all referenced tables have been processed
                let can_process = table.foreign_keys.iter().all(|fk| {
                    processed_tables.contains(&fk.referenced_table) || fk.referenced_table == table.name
                });

                if can_process {
                    if let Some(rows) = table_data.get(&table.name) {
                        if !rows.is_empty() {
                            match self.generate_table_insert_statements(table, rows, warnings) {
                                Ok(table_statements) => {
                                    statements.extend(table_statements);
                                }
                                Err(e) => {
                                    warnings.push(format!("Failed to generate INSERT for table {}: {}", table.name, e));
                                }
                            }
                        }
                    }
                    processed_tables.insert(table.name.clone());
                    progress_made = true;
                    false // Remove from remaining_tables
                } else {
                    true // Keep in remaining_tables
                }
            });

            if !progress_made && !remaining_tables.is_empty() {
                return Err(FireupError::TypeMapping(
                    "Circular dependency detected in table relationships".to_string()
                ));
            }
        }

        Ok(statements)
    }

    /// Generate INSERT statements for a single table
    fn generate_table_insert_statements(
        &mut self,
        table: &TableDefinition,
        rows: &[TableRow],
        warnings: &mut Vec<String>,
    ) -> Result<Vec<SQLStatement>, FireupError> {
        let mut statements = Vec::new();
        
        if rows.is_empty() {
            return Ok(statements);
        }

        let column_names: Vec<String> = table.columns.iter().map(|c| c.name.clone()).collect();
        
        // Process rows in batches
        for batch in rows.chunks(self.config.batch_size) {
            let statement = if self.config.use_parameters {
                self.generate_parameterized_insert(table, &column_names, batch, warnings)?
            } else {
                self.generate_literal_insert(table, &column_names, batch, warnings)?
            };
            
            statements.push(statement);
        }

        Ok(statements)
    }

    /// Generate parameterized INSERT statement
    pub fn generate_parameterized_insert(
        &mut self,
        table: &TableDefinition,
        column_names: &[String],
        rows: &[TableRow],
        _warnings: &mut Vec<String>,
    ) -> Result<SQLStatement, FireupError> {
        let mut sql = format!("INSERT INTO {} ({})", table.name, column_names.join(", "));
        
        // Add conflict handling if configured
        if self.config.handle_conflicts {
            match self.config.conflict_strategy {
                ConflictStrategy::Ignore => {
                    sql.push_str(" ON CONFLICT DO NOTHING");
                }
                ConflictStrategy::Update => {
                    if let Some(ref pk) = table.primary_key {
                        sql.push_str(&format!(" ON CONFLICT ({}) DO UPDATE SET ", pk.columns.join(", ")));
                        let updates: Vec<String> = column_names.iter()
                            .filter(|col| !pk.columns.contains(col))
                            .map(|col| format!("{} = EXCLUDED.{}", col, col))
                            .collect();
                        sql.push_str(&updates.join(", "));
                    }
                }
                ConflictStrategy::Fail => {
                    // No conflict handling - use default PostgreSQL behavior
                }
            }
        }
        
        sql.push_str(" VALUES ");
        
        let mut parameters = Vec::new();
        let mut value_clauses = Vec::new();
        
        for row in rows {
            let mut row_params = Vec::new();
            for column_name in column_names {
                if let Some(value) = row.columns.get(column_name) {
                    self.parameter_counter += 1;
                    row_params.push(format!("${}", self.parameter_counter));
                    parameters.push(value.clone());
                } else {
                    row_params.push("NULL".to_string());
                }
            }
            value_clauses.push(format!("({})", row_params.join(", ")));
        }
        
        sql.push_str(&value_clauses.join(", "));
        sql.push(';');
        
        Ok(SQLStatement {
            sql,
            parameters,
            table_name: table.name.clone(),
            row_count: rows.len(),
            statement_type: StatementType::Insert,
        })
    }

    /// Generate literal INSERT statement (no parameters)
    pub fn generate_literal_insert(
        &self,
        table: &TableDefinition,
        column_names: &[String],
        rows: &[TableRow],
        _warnings: &mut Vec<String>,
    ) -> Result<SQLStatement, FireupError> {
        let mut sql = format!("INSERT INTO {} ({})", table.name, column_names.join(", "));
        
        // Add conflict handling if configured
        if self.config.handle_conflicts {
            match self.config.conflict_strategy {
                ConflictStrategy::Ignore => {
                    sql.push_str(" ON CONFLICT DO NOTHING");
                }
                ConflictStrategy::Update => {
                    if let Some(ref pk) = table.primary_key {
                        sql.push_str(&format!(" ON CONFLICT ({}) DO UPDATE SET ", pk.columns.join(", ")));
                        let updates: Vec<String> = column_names.iter()
                            .filter(|col| !pk.columns.contains(col))
                            .map(|col| format!("{} = EXCLUDED.{}", col, col))
                            .collect();
                        sql.push_str(&updates.join(", "));
                    }
                }
                ConflictStrategy::Fail => {
                    // No conflict handling
                }
            }
        }
        
        sql.push_str(" VALUES ");
        
        let mut value_clauses = Vec::new();
        for row in rows {
            let mut values = Vec::new();
            for column_name in column_names {
                if let Some(value) = row.columns.get(column_name) {
                    values.push(self.format_sql_value(value));
                } else {
                    values.push("NULL".to_string());
                }
            }
            value_clauses.push(format!("({})", values.join(", ")));
        }
        
        sql.push_str(&value_clauses.join(",\n"));
        sql.push(';');
        
        Ok(SQLStatement {
            sql,
            parameters: vec![],
            table_name: table.name.clone(),
            row_count: rows.len(),
            statement_type: StatementType::Insert,
        })
    }

    /// Generate CREATE INDEX statements for a table
    pub fn generate_index_statements(&self, table: &TableDefinition) -> Result<Vec<SQLStatement>, FireupError> {
        let mut statements = Vec::new();
        
        for index in &table.indexes {
            let unique_clause = if index.unique { "UNIQUE " } else { "" };
            let index_type = index.index_type.as_deref().unwrap_or("BTREE");
            
            let sql = format!(
                "CREATE {}INDEX IF NOT EXISTS {} ON {} USING {} ({});",
                unique_clause,
                index.name,
                table.name,
                index_type,
                index.columns.join(", ")
            );
            
            statements.push(SQLStatement {
                sql,
                parameters: vec![],
                table_name: table.name.clone(),
                row_count: 0,
                statement_type: StatementType::CreateIndex,
            });
        }
        
        Ok(statements)
    }

    /// Generate ALTER TABLE statements for foreign key constraints
    pub fn generate_constraint_statements(&self, schema: &NormalizedSchema) -> Result<Vec<SQLStatement>, FireupError> {
        let mut statements = Vec::new();
        
        for table in &schema.tables {
            for fk in &table.foreign_keys {
                let sql = format!(
                    "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({});",
                    table.name,
                    fk.constraint_name,
                    fk.column,
                    fk.referenced_table,
                    fk.referenced_column
                );
                
                statements.push(SQLStatement {
                    sql,
                    parameters: vec![],
                    table_name: table.name.clone(),
                    row_count: 0,
                    statement_type: StatementType::AlterTable,
                });
            }
        }
        
        Ok(statements)
    }

    /// Format a JSON value for SQL insertion
    pub fn format_sql_value(&self, value: &Value) -> String {
        match value {
            Value::Null => "NULL".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => {
                // Escape single quotes and wrap in quotes
                format!("'{}'", s.replace('\'', "''"))
            }
            Value::Array(_) | Value::Object(_) => {
                // For complex types, serialize to JSON and escape
                let json_str = value.to_string();
                format!("'{}'", json_str.replace('\'', "''"))
            }
        }
    }

    /// Generate bulk INSERT statements for large datasets
    pub fn generate_bulk_insert(
        &mut self,
        table: &TableDefinition,
        rows: &[TableRow],
        batch_size: Option<usize>,
    ) -> Result<Vec<SQLStatement>, FireupError> {
        let effective_batch_size = batch_size.unwrap_or(self.config.batch_size);
        let mut statements = Vec::new();
        let mut warnings = Vec::new();

        let column_names: Vec<String> = table.columns.iter().map(|c| c.name.clone()).collect();

        for batch in rows.chunks(effective_batch_size) {
            let statement = self.generate_table_insert_statements(table, batch, &mut warnings)?;
            statements.extend(statement);
        }

        Ok(statements)
    }

    /// Generate COPY statement for very large datasets (PostgreSQL specific)
    pub fn generate_copy_statement(
        &self,
        table: &TableDefinition,
        csv_file_path: &str,
    ) -> Result<SQLStatement, FireupError> {
        let column_names: Vec<String> = table.columns.iter().map(|c| c.name.clone()).collect();
        
        let sql = format!(
            "COPY {} ({}) FROM '{}' WITH (FORMAT csv, HEADER true, DELIMITER ',', QUOTE '\"', ESCAPE '\"');",
            table.name,
            column_names.join(", "),
            csv_file_path
        );

        Ok(SQLStatement {
            sql,
            parameters: vec![],
            table_name: table.name.clone(),
            row_count: 0, // Unknown for COPY statements
            statement_type: StatementType::Insert,
        })
    }

    /// Reset parameter counter (useful for multiple generation runs)
    pub fn reset_parameter_counter(&mut self) {
        self.parameter_counter = 0;
    }

    /// Get current parameter counter value
    pub fn get_parameter_count(&self) -> u32 {
        self.parameter_counter
    }
}

impl Default for SQLGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Utility functions for SQL generation
pub mod utils {
    use super::*;

    /// Escape SQL identifier (table name, column name, etc.)
    pub fn escape_identifier(identifier: &str) -> String {
        if identifier.chars().all(|c| c.is_alphanumeric() || c == '_') 
            && !identifier.chars().next().unwrap_or('0').is_ascii_digit() {
            identifier.to_string()
        } else {
            format!("\"{}\"", identifier.replace('"', "\"\""))
        }
    }

    /// Validate SQL identifier
    pub fn is_valid_identifier(identifier: &str) -> bool {
        !identifier.is_empty() 
            && identifier.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !identifier.chars().next().unwrap_or('0').is_ascii_digit()
    }

    /// Generate a safe table name from a collection name
    pub fn sanitize_table_name(collection_name: &str) -> String {
        let sanitized = collection_name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
            .collect::<String>();
        
        // Ensure it doesn't start with a number
        if sanitized.chars().next().unwrap_or('a').is_ascii_digit() {
            format!("table_{}", sanitized)
        } else {
            sanitized
        }
    }

    /// Generate a constraint name
    pub fn generate_constraint_name(table: &str, constraint_type: &str, columns: &[String]) -> String {
        let column_part = if columns.len() == 1 {
            columns[0].clone()
        } else {
            columns.join("_")
        };
        
        format!("{}_{}_{}_{}", table, constraint_type, column_part, 
                Uuid::new_v4().to_string().split('-').next().unwrap_or("0000"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use crate::data_importer::transformer::*;
    use serde_json::json;

    fn create_test_table() -> TableDefinition {
        let mut table = TableDefinition::new("test_table".to_string());
        table.add_column(ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid).not_null());
        table.add_column(ColumnDefinition::new("name".to_string(), PostgreSQLType::Varchar(Some(255))));
        table.add_column(ColumnDefinition::new("age".to_string(), PostgreSQLType::Integer));
        table.set_primary_key(PrimaryKeyDefinition {
            name: "test_table_pkey".to_string(),
            columns: vec!["id".to_string()],
        });
        table
    }

    fn create_test_rows() -> Vec<TableRow> {
        vec![
            TableRow {
                columns: [
                    ("id".to_string(), json!("550e8400-e29b-41d4-a716-446655440000")),
                    ("name".to_string(), json!("John Doe")),
                    ("age".to_string(), json!(30)),
                ].iter().cloned().collect(),
                primary_key: json!("550e8400-e29b-41d4-a716-446655440000"),
                foreign_keys: HashMap::new(),
            },
            TableRow {
                columns: [
                    ("id".to_string(), json!("550e8400-e29b-41d4-a716-446655440001")),
                    ("name".to_string(), json!("Jane Smith")),
                    ("age".to_string(), json!(25)),
                ].iter().cloned().collect(),
                primary_key: json!("550e8400-e29b-41d4-a716-446655440001"),
                foreign_keys: HashMap::new(),
            },
        ]
    }

    #[test]
    fn test_generate_create_table_statement() {
        let generator = SQLGenerator::new();
        let table = create_test_table();

        let statement = generator.generate_create_table_statement(&table).unwrap();

        assert!(statement.sql.contains("CREATE TABLE IF NOT EXISTS test_table"));
        assert!(statement.sql.contains("id UUID NOT NULL"));
        assert!(statement.sql.contains("name VARCHAR(255)"));
        assert!(statement.sql.contains("PRIMARY KEY (id)"));
    }

    #[test]
    fn test_generate_literal_insert() {
        let generator = SQLGenerator::new();
        let table = create_test_table();
        let rows = create_test_rows();
        let column_names: Vec<String> = table.columns.iter().map(|c| c.name.clone()).collect();
        let mut warnings = Vec::new();

        let statement = generator.generate_literal_insert(&table, &column_names, &rows, &mut warnings).unwrap();

        assert!(statement.sql.contains("INSERT INTO test_table"));
        assert!(statement.sql.contains("John Doe"));
        assert!(statement.sql.contains("Jane Smith"));
        assert_eq!(statement.row_count, 2);
    }

    #[test]
    fn test_generate_parameterized_insert() {
        let mut generator = SQLGenerator::new();
        let table = create_test_table();
        let rows = create_test_rows();
        let column_names: Vec<String> = table.columns.iter().map(|c| c.name.clone()).collect();
        let mut warnings = Vec::new();

        let statement = generator.generate_parameterized_insert(&table, &column_names, &rows, &mut warnings).unwrap();

        assert!(statement.sql.contains("INSERT INTO test_table"));
        assert!(statement.sql.contains("$1"));
        assert!(!statement.parameters.is_empty());
        assert_eq!(statement.row_count, 2);
    }

    #[test]
    fn test_format_sql_value() {
        let generator = SQLGenerator::new();

        assert_eq!(generator.format_sql_value(&json!(null)), "NULL");
        assert_eq!(generator.format_sql_value(&json!(true)), "true");
        assert_eq!(generator.format_sql_value(&json!(42)), "42");
        assert_eq!(generator.format_sql_value(&json!("test")), "'test'");
        assert_eq!(generator.format_sql_value(&json!("test's")), "'test''s'");
    }

    #[test]
    fn test_conflict_strategies() {
        let config = SQLGenerationConfig {
            handle_conflicts: true,
            conflict_strategy: ConflictStrategy::Ignore,
            ..Default::default()
        };
        let generator = SQLGenerator::with_config(config);
        let table = create_test_table();
        let rows = create_test_rows();
        let column_names: Vec<String> = table.columns.iter().map(|c| c.name.clone()).collect();
        let mut warnings = Vec::new();

        let statement = generator.generate_literal_insert(&table, &column_names, &rows, &mut warnings).unwrap();

        assert!(statement.sql.contains("ON CONFLICT DO NOTHING"));
    }

    #[test]
    fn test_utils_sanitize_table_name() {
        assert_eq!(utils::sanitize_table_name("valid_name"), "valid_name");
        assert_eq!(utils::sanitize_table_name("invalid-name"), "invalid_name");
        assert_eq!(utils::sanitize_table_name("123invalid"), "table_123invalid");
    }

    #[test]
    fn test_utils_is_valid_identifier() {
        assert!(utils::is_valid_identifier("valid_name"));
        assert!(!utils::is_valid_identifier("invalid-name"));
        assert!(!utils::is_valid_identifier("123invalid"));
        assert!(!utils::is_valid_identifier(""));
    }
}