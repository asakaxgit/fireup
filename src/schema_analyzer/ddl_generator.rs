use crate::types::{
    NormalizedSchema, TableDefinition, ColumnDefinition, ForeignKeyDefinition, 
    IndexDefinition, Constraint, ConstraintType, SchemaWarning
};
use crate::error::{FireupResult, FireupError};
use std::collections::HashMap;

/// DDL generator for creating PostgreSQL schema statements
pub struct DDLGenerator {
    /// Configuration options for DDL generation
    config: DDLConfig,
}

/// Configuration options for DDL generation
#[derive(Debug, Clone)]
pub struct DDLConfig {
    /// Whether to include IF NOT EXISTS clauses
    pub include_if_not_exists: bool,
    /// Whether to include comments in generated DDL
    pub include_comments: bool,
    /// Schema name to use for tables
    pub schema_name: Option<String>,
    /// Whether to generate DROP statements before CREATE
    pub include_drop_statements: bool,
}

/// Generated DDL statements for a complete schema
#[derive(Debug, Clone)]
pub struct GeneratedDDL {
    /// CREATE TABLE statements
    pub table_statements: Vec<String>,
    /// ALTER TABLE statements for foreign keys
    pub foreign_key_statements: Vec<String>,
    /// CREATE INDEX statements
    pub index_statements: Vec<String>,
    /// Additional constraint statements
    pub constraint_statements: Vec<String>,
    /// DROP statements (if requested)
    pub drop_statements: Vec<String>,
    /// Comments and documentation
    pub comments: Vec<String>,
    /// Warnings generated during DDL creation
    pub warnings: Vec<SchemaWarning>,
}

impl Default for DDLConfig {
    fn default() -> Self {
        Self {
            include_if_not_exists: true,
            include_comments: true,
            schema_name: None,
            include_drop_statements: false,
        }
    }
}

impl DDLGenerator {
    /// Create a new DDL generator with default configuration
    pub fn new() -> Self {
        Self {
            config: DDLConfig::default(),
        }
    }

    /// Create a new DDL generator with custom configuration
    pub fn with_config(config: DDLConfig) -> Self {
        Self { config }
    }

    /// Generate complete DDL from normalized schema
    pub fn generate_ddl(&self, schema: &NormalizedSchema) -> FireupResult<GeneratedDDL> {
        let mut ddl = GeneratedDDL {
            table_statements: Vec::new(),
            foreign_key_statements: Vec::new(),
            index_statements: Vec::new(),
            constraint_statements: Vec::new(),
            drop_statements: Vec::new(),
            comments: Vec::new(),
            warnings: schema.warnings.clone(),
        };

        // Add header comment
        if self.config.include_comments {
            ddl.comments.push(self.generate_header_comment(schema));
        }

        // Generate DROP statements if requested
        if self.config.include_drop_statements {
            ddl.drop_statements = self.generate_drop_statements(&schema.tables)?;
        }

        // Generate CREATE TABLE statements
        for table in &schema.tables {
            let table_ddl = self.generate_table_ddl(table)?;
            ddl.table_statements.push(table_ddl);

            // Generate foreign key statements (added after table creation)
            for fk in &table.foreign_keys {
                let fk_ddl = self.generate_foreign_key_ddl(&table.name, fk)?;
                ddl.foreign_key_statements.push(fk_ddl);
            }

            // Generate index statements
            for index in &table.indexes {
                let index_ddl = self.generate_index_ddl(&table.name, index)?;
                ddl.index_statements.push(index_ddl);
            }
        }

        // Generate additional constraint statements
        for constraint in &schema.constraints {
            let constraint_ddl = self.generate_constraint_ddl(constraint)?;
            ddl.constraint_statements.push(constraint_ddl);
        }

        Ok(ddl)
    }

    /// Generate CREATE TABLE statement for a single table
    pub fn generate_table_ddl(&self, table: &TableDefinition) -> FireupResult<String> {
        let mut ddl = String::new();

        // Add table comment if enabled
        if self.config.include_comments {
            ddl.push_str(&format!("-- Table: {}\n", table.name));
        }

        // Start CREATE TABLE statement
        let table_name = self.format_table_name(&table.name);
        if self.config.include_if_not_exists {
            ddl.push_str(&format!("CREATE TABLE IF NOT EXISTS {} (\n", table_name));
        } else {
            ddl.push_str(&format!("CREATE TABLE {} (\n", table_name));
        }

        // Add columns
        let column_definitions: Vec<String> = table.columns
            .iter()
            .map(|col| self.generate_column_definition(col))
            .collect::<FireupResult<Vec<_>>>()?;

        ddl.push_str(&format!("    {}", column_definitions.join(",\n    ")));

        // Add primary key constraint if specified
        if !table.primary_key.is_empty() {
            ddl.push_str(",\n    ");
            ddl.push_str(&format!(
                "CONSTRAINT pk_{} PRIMARY KEY ({})",
                table.name.to_lowercase(),
                table.primary_key.join(", ")
            ));
        }

        ddl.push_str("\n);\n\n");

        Ok(ddl)
    }

    /// Generate column definition string
    fn generate_column_definition(&self, column: &ColumnDefinition) -> FireupResult<String> {
        let mut definition = format!("{} {}", column.name, column.column_type.to_sql());

        // Add NOT NULL constraint
        if !column.nullable {
            definition.push_str(" NOT NULL");
        }

        // Add default value
        if let Some(default) = &column.default_value {
            definition.push_str(&format!(" DEFAULT {}", self.format_default_value(default)?));
        }

        // Add additional constraints
        for constraint in &column.constraints {
            definition.push_str(&format!(" {}", constraint));
        }

        Ok(definition)
    }

    /// Generate foreign key constraint statement
    fn generate_foreign_key_ddl(&self, table_name: &str, fk: &ForeignKeyDefinition) -> FireupResult<String> {
        let table_name = self.format_table_name(table_name);
        let referenced_table = self.format_table_name(&fk.referenced_table);
        
        Ok(format!(
            "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({});",
            table_name,
            fk.constraint_name,
            fk.column,
            referenced_table,
            fk.referenced_column
        ))
    }

    /// Generate index creation statement
    fn generate_index_ddl(&self, table_name: &str, index: &IndexDefinition) -> FireupResult<String> {
        let table_name = self.format_table_name(table_name);
        let unique_clause = if index.unique { "UNIQUE " } else { "" };
        let index_type = index.index_type.as_deref().unwrap_or("BTREE");
        
        Ok(format!(
            "CREATE {}INDEX {} ON {} USING {} ({});",
            unique_clause,
            index.name,
            table_name,
            index_type,
            index.columns.join(", ")
        ))
    }

    /// Generate constraint statement
    fn generate_constraint_ddl(&self, constraint: &Constraint) -> FireupResult<String> {
        let table_name = self.format_table_name(&constraint.table);
        
        match constraint.constraint_type {
            ConstraintType::NotNull => {
                // NOT NULL constraints are handled in column definitions
                Ok(String::new())
            }
            ConstraintType::Unique => {
                Ok(format!(
                    "ALTER TABLE {} ADD CONSTRAINT {} UNIQUE ({});",
                    table_name,
                    constraint.name,
                    constraint.columns.join(", ")
                ))
            }
            ConstraintType::Check => {
                let check_condition = constraint.parameters.get("condition")
                    .ok_or_else(|| FireupError::schema_analysis(
                        "Check constraint missing condition parameter",
                        None,
                        None,
                        FireupError::new_context("generate_constraint_ddl")
                    ))?;
                Ok(format!(
                    "ALTER TABLE {} ADD CONSTRAINT {} CHECK ({});",
                    table_name,
                    constraint.name,
                    check_condition
                ))
            }
            ConstraintType::PrimaryKey => {
                Ok(format!(
                    "ALTER TABLE {} ADD CONSTRAINT {} PRIMARY KEY ({});",
                    table_name,
                    constraint.name,
                    constraint.columns.join(", ")
                ))
            }
            ConstraintType::ForeignKey => {
                let referenced_table = constraint.parameters.get("referenced_table")
                    .ok_or_else(|| FireupError::schema_analysis(
                        "Foreign key constraint missing referenced_table parameter",
                        None,
                        None,
                        FireupError::new_context("generate_constraint_ddl")
                    ))?;
                let referenced_column = constraint.parameters.get("referenced_column")
                    .ok_or_else(|| FireupError::schema_analysis(
                        "Foreign key constraint missing referenced_column parameter",
                        None,
                        None,
                        FireupError::new_context("generate_constraint_ddl")
                    ))?;
                
                Ok(format!(
                    "ALTER TABLE {} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({});",
                    table_name,
                    constraint.name,
                    constraint.columns.join(", "),
                    self.format_table_name(referenced_table),
                    referenced_column
                ))
            }
        }
    }

    /// Generate DROP statements for tables
    fn generate_drop_statements(&self, tables: &[TableDefinition]) -> FireupResult<Vec<String>> {
        let mut statements = Vec::new();
        
        // Drop tables in reverse order to handle foreign key dependencies
        for table in tables.iter().rev() {
            let table_name = self.format_table_name(&table.name);
            statements.push(format!("DROP TABLE IF EXISTS {} CASCADE;", table_name));
        }
        
        Ok(statements)
    }

    /// Generate header comment for the DDL file
    fn generate_header_comment(&self, schema: &NormalizedSchema) -> String {
        format!(
            "-- PostgreSQL DDL generated from Firestore schema analysis\n\
             -- Generated at: {}\n\
             -- Tables: {}\n\
             -- Relationships: {}\n\
             -- Schema version: {}\n",
            schema.metadata.generated_at.format("%Y-%m-%d %H:%M:%S UTC"),
            schema.metadata.table_count,
            schema.metadata.relationship_count,
            schema.metadata.version
        )
    }

    /// Format table name with optional schema prefix
    fn format_table_name(&self, table_name: &str) -> String {
        if let Some(schema) = &self.config.schema_name {
            format!("{}.{}", schema, table_name)
        } else {
            table_name.to_string()
        }
    }

    /// Format default value for SQL
    fn format_default_value(&self, value: &serde_json::Value) -> FireupResult<String> {
        match value {
            serde_json::Value::String(s) => Ok(format!("'{}'", s.replace('\'', "''"))),
            serde_json::Value::Number(n) => Ok(n.to_string()),
            serde_json::Value::Bool(b) => Ok(b.to_string().to_uppercase()),
            serde_json::Value::Null => Ok("NULL".to_string()),
            _ => Err(FireupError::schema_analysis(
                format!("Unsupported default value type: {:?}", value),
                None,
                None,
                FireupError::new_context("format_default_value")
            )),
        }
    }
}

impl GeneratedDDL {
    /// Get all DDL statements in execution order
    pub fn all_statements(&self) -> Vec<String> {
        let mut statements = Vec::new();
        
        // Add comments first
        statements.extend(self.comments.clone());
        
        // Add drop statements
        statements.extend(self.drop_statements.clone());
        
        // Add table creation statements
        statements.extend(self.table_statements.clone());
        
        // Add foreign key statements
        statements.extend(self.foreign_key_statements.clone());
        
        // Add additional constraint statements
        statements.extend(self.constraint_statements.clone());
        
        // Add index statements last
        statements.extend(self.index_statements.clone());
        
        statements
    }

    /// Get complete DDL as a single string
    pub fn to_string(&self) -> String {
        self.all_statements().join("\n")
    }

    /// Get summary of generated DDL
    pub fn summary(&self) -> DDLSummary {
        DDLSummary {
            table_count: self.table_statements.len(),
            foreign_key_count: self.foreign_key_statements.len(),
            index_count: self.index_statements.len(),
            constraint_count: self.constraint_statements.len(),
            warning_count: self.warnings.len(),
        }
    }
}

/// Summary of generated DDL statements
#[derive(Debug, Clone)]
pub struct DDLSummary {
    pub table_count: usize,
    pub foreign_key_count: usize,
    pub index_count: usize,
    pub constraint_count: usize,
    pub warning_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn create_test_table() -> TableDefinition {
        let mut table = TableDefinition::new("users".to_string());
        
        table.add_column(ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid).not_null());
        table.add_column(ColumnDefinition::new("email".to_string(), PostgreSQLType::Varchar(Some(255))).not_null());
        table.add_column(ColumnDefinition::new("name".to_string(), PostgreSQLType::Text));
        table.add_column(ColumnDefinition::new("created_at".to_string(), PostgreSQLType::Timestamp).not_null());
        
        table.set_primary_key(vec!["id".to_string()]);
        
        table.add_foreign_key(ForeignKeyDefinition {
            column: "profile_id".to_string(),
            referenced_table: "profiles".to_string(),
            referenced_column: "id".to_string(),
            constraint_name: "fk_users_profile".to_string(),
        });
        
        table.add_index(IndexDefinition {
            name: "idx_users_email".to_string(),
            columns: vec!["email".to_string()],
            unique: true,
            index_type: Some("BTREE".to_string()),
        });
        
        table
    }

    fn create_test_schema() -> NormalizedSchema {
        NormalizedSchema {
            tables: vec![create_test_table()],
            relationships: Vec::new(),
            constraints: Vec::new(),
            warnings: Vec::new(),
            metadata: SchemaMetadata {
                generated_at: Utc::now(),
                source_analysis_id: Uuid::new_v4(),
                version: "1.0.0".to_string(),
                table_count: 1,
                relationship_count: 0,
            },
        }
    }

    #[test]
    fn test_generate_table_ddl() {
        let generator = DDLGenerator::new();
        let table = create_test_table();
        
        let ddl = generator.generate_table_ddl(&table).unwrap();
        
        assert!(ddl.contains("CREATE TABLE IF NOT EXISTS users"));
        assert!(ddl.contains("id UUID NOT NULL"));
        assert!(ddl.contains("email VARCHAR(255) NOT NULL"));
        assert!(ddl.contains("CONSTRAINT pk_users PRIMARY KEY (id)"));
    }

    #[test]
    fn test_generate_complete_ddl() {
        let generator = DDLGenerator::new();
        let schema = create_test_schema();
        
        let ddl = generator.generate_ddl(&schema).unwrap();
        
        assert_eq!(ddl.table_statements.len(), 1);
        assert_eq!(ddl.foreign_key_statements.len(), 1);
        assert_eq!(ddl.index_statements.len(), 1);
        
        let complete_ddl = ddl.to_string();
        assert!(complete_ddl.contains("CREATE TABLE"));
        assert!(complete_ddl.contains("ALTER TABLE"));
        assert!(complete_ddl.contains("CREATE UNIQUE INDEX"));
    }

    #[test]
    fn test_ddl_config_options() {
        let config = DDLConfig {
            include_if_not_exists: false,
            include_comments: false,
            schema_name: Some("firestore".to_string()),
            include_drop_statements: true,
        };
        
        let generator = DDLGenerator::with_config(config);
        let schema = create_test_schema();
        
        let ddl = generator.generate_ddl(&schema).unwrap();
        
        // Should not include IF NOT EXISTS
        assert!(!ddl.table_statements[0].contains("IF NOT EXISTS"));
        
        // Should include schema name
        assert!(ddl.table_statements[0].contains("firestore.users"));
        
        // Should include drop statements
        assert!(!ddl.drop_statements.is_empty());
        assert!(ddl.drop_statements[0].contains("DROP TABLE"));
    }
}