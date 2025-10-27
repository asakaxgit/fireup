use crate::types::{
    NormalizedSchema, TableDefinition, ColumnDefinition, ForeignKeyDefinition, 
    IndexDefinition, Constraint, ConstraintType, SchemaWarning
};
use crate::error::{FireupResult, FireupError};

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
    use std::collections::HashMap;

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

    fn create_complex_test_schema() -> NormalizedSchema {
        let mut users_table = TableDefinition::new("users".to_string());
        users_table.add_column(ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid).not_null());
        users_table.add_column(ColumnDefinition::new("email".to_string(), PostgreSQLType::Varchar(Some(255))).not_null());
        users_table.add_column(ColumnDefinition::new("age".to_string(), PostgreSQLType::Integer));
        users_table.add_column(ColumnDefinition::new("score".to_string(), PostgreSQLType::Numeric(Some(10), Some(2))).with_default(serde_json::json!(0.0)));
        users_table.add_column(ColumnDefinition::new("active".to_string(), PostgreSQLType::Boolean).with_default(serde_json::json!(true)));
        users_table.set_primary_key(vec!["id".to_string()]);

        let mut posts_table = TableDefinition::new("posts".to_string());
        posts_table.add_column(ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid).not_null());
        posts_table.add_column(ColumnDefinition::new("title".to_string(), PostgreSQLType::Text).not_null());
        posts_table.add_column(ColumnDefinition::new("content".to_string(), PostgreSQLType::Text));
        posts_table.add_column(ColumnDefinition::new("user_id".to_string(), PostgreSQLType::Uuid).not_null());
        posts_table.add_column(ColumnDefinition::new("metadata".to_string(), PostgreSQLType::Jsonb));
        posts_table.add_column(ColumnDefinition::new("tags".to_string(), PostgreSQLType::Array(Box::new(PostgreSQLType::Text))));
        posts_table.set_primary_key(vec!["id".to_string()]);
        
        posts_table.add_foreign_key(ForeignKeyDefinition {
            column: "user_id".to_string(),
            referenced_table: "users".to_string(),
            referenced_column: "id".to_string(),
            constraint_name: "fk_posts_user".to_string(),
        });

        posts_table.add_index(IndexDefinition {
            name: "idx_posts_user_id".to_string(),
            columns: vec!["user_id".to_string()],
            unique: false,
            index_type: Some("BTREE".to_string()),
        });

        posts_table.add_index(IndexDefinition {
            name: "idx_posts_title".to_string(),
            columns: vec!["title".to_string()],
            unique: false,
            index_type: Some("BTREE".to_string()),
        });

        // Add constraints
        let mut constraints = Vec::new();
        
        // Unique constraint
        constraints.push(Constraint {
            name: "uq_users_email".to_string(),
            table: "users".to_string(),
            constraint_type: ConstraintType::Unique,
            columns: vec!["email".to_string()],
            parameters: HashMap::new(),
        });

        // Check constraint
        let mut check_params = HashMap::new();
        check_params.insert("condition".to_string(), "age >= 0 AND age <= 150".to_string());
        constraints.push(Constraint {
            name: "chk_users_age_range".to_string(),
            table: "users".to_string(),
            constraint_type: ConstraintType::Check,
            columns: vec!["age".to_string()],
            parameters: check_params,
        });

        NormalizedSchema {
            tables: vec![users_table, posts_table],
            relationships: Vec::new(),
            constraints,
            warnings: vec![
                SchemaWarning {
                    level: WarningLevel::Warning,
                    message: "Consider adding index on frequently queried column".to_string(),
                    context: "posts.created_at".to_string(),
                    suggestion: Some("CREATE INDEX idx_posts_created_at ON posts (created_at)".to_string()),
                }
            ],
            metadata: SchemaMetadata {
                generated_at: Utc::now(),
                source_analysis_id: Uuid::new_v4(),
                version: "1.0.0".to_string(),
                table_count: 2,
                relationship_count: 1,
            },
        }
    }

    #[test]
    fn test_generate_table_ddl_basic() {
        let generator = DDLGenerator::new();
        let table = create_test_table();
        
        let ddl = generator.generate_table_ddl(&table).unwrap();
        
        assert!(ddl.contains("CREATE TABLE IF NOT EXISTS users"));
        assert!(ddl.contains("id UUID NOT NULL"));
        assert!(ddl.contains("email VARCHAR(255) NOT NULL"));
        assert!(ddl.contains("name TEXT"));
        assert!(ddl.contains("created_at TIMESTAMP WITH TIME ZONE NOT NULL"));
        assert!(ddl.contains("CONSTRAINT pk_users PRIMARY KEY (id)"));
    }

    #[test]
    fn test_generate_table_ddl_with_defaults_and_constraints() {
        let generator = DDLGenerator::new();
        let mut table = TableDefinition::new("test_table".to_string());
        
        table.add_column(
            ColumnDefinition::new("id".to_string(), PostgreSQLType::Integer)
                .not_null()
                .add_constraint("UNIQUE".to_string())
        );
        table.add_column(
            ColumnDefinition::new("status".to_string(), PostgreSQLType::Varchar(Some(50)))
                .with_default(serde_json::json!("active"))
        );
        table.add_column(
            ColumnDefinition::new("count".to_string(), PostgreSQLType::Integer)
                .with_default(serde_json::json!(0))
        );
        table.add_column(
            ColumnDefinition::new("enabled".to_string(), PostgreSQLType::Boolean)
                .with_default(serde_json::json!(true))
        );
        
        let ddl = generator.generate_table_ddl(&table).unwrap();
        
        assert!(ddl.contains("id INTEGER NOT NULL UNIQUE"));
        assert!(ddl.contains("status VARCHAR(50) DEFAULT 'active'"));
        assert!(ddl.contains("count INTEGER DEFAULT 0"));
        assert!(ddl.contains("enabled BOOLEAN DEFAULT TRUE"));
    }

    #[test]
    fn test_generate_table_ddl_complex_types() {
        let generator = DDLGenerator::new();
        let mut table = TableDefinition::new("complex_table".to_string());
        
        table.add_column(ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid).not_null());
        table.add_column(ColumnDefinition::new("price".to_string(), PostgreSQLType::Numeric(Some(10), Some(2))));
        table.add_column(ColumnDefinition::new("data".to_string(), PostgreSQLType::Jsonb));
        table.add_column(ColumnDefinition::new("tags".to_string(), PostgreSQLType::Array(Box::new(PostgreSQLType::Text))));
        table.add_column(ColumnDefinition::new("scores".to_string(), PostgreSQLType::Array(Box::new(PostgreSQLType::Integer))));
        
        let ddl = generator.generate_table_ddl(&table).unwrap();
        
        assert!(ddl.contains("id UUID NOT NULL"));
        assert!(ddl.contains("price NUMERIC(10, 2)"));
        assert!(ddl.contains("data JSONB"));
        assert!(ddl.contains("tags TEXT[]"));
        assert!(ddl.contains("scores INTEGER[]"));
    }

    #[test]
    fn test_generate_foreign_key_ddl() {
        let generator = DDLGenerator::new();
        let fk = ForeignKeyDefinition {
            column: "user_id".to_string(),
            referenced_table: "users".to_string(),
            referenced_column: "id".to_string(),
            constraint_name: "fk_posts_user".to_string(),
        };
        
        let ddl = generator.generate_foreign_key_ddl("posts", &fk).unwrap();
        
        assert!(ddl.contains("ALTER TABLE posts"));
        assert!(ddl.contains("ADD CONSTRAINT fk_posts_user"));
        assert!(ddl.contains("FOREIGN KEY (user_id)"));
        assert!(ddl.contains("REFERENCES users (id)"));
    }

    #[test]
    fn test_generate_index_ddl() {
        let generator = DDLGenerator::new();
        
        // Test unique index
        let unique_index = IndexDefinition {
            name: "idx_users_email".to_string(),
            columns: vec!["email".to_string()],
            unique: true,
            index_type: Some("BTREE".to_string()),
        };
        
        let ddl = generator.generate_index_ddl("users", &unique_index).unwrap();
        assert!(ddl.contains("CREATE UNIQUE INDEX idx_users_email"));
        assert!(ddl.contains("ON users USING BTREE"));
        assert!(ddl.contains("(email)"));

        // Test composite index
        let composite_index = IndexDefinition {
            name: "idx_posts_user_created".to_string(),
            columns: vec!["user_id".to_string(), "created_at".to_string()],
            unique: false,
            index_type: Some("BTREE".to_string()),
        };
        
        let ddl = generator.generate_index_ddl("posts", &composite_index).unwrap();
        assert!(ddl.contains("CREATE INDEX idx_posts_user_created"));
        assert!(ddl.contains("(user_id, created_at)"));
    }

    #[test]
    fn test_generate_constraint_ddl() {
        let generator = DDLGenerator::new();
        
        // Test unique constraint
        let unique_constraint = Constraint {
            name: "uq_users_email".to_string(),
            table: "users".to_string(),
            constraint_type: ConstraintType::Unique,
            columns: vec!["email".to_string()],
            parameters: HashMap::new(),
        };
        
        let ddl = generator.generate_constraint_ddl(&unique_constraint).unwrap();
        assert!(ddl.contains("ALTER TABLE users"));
        assert!(ddl.contains("ADD CONSTRAINT uq_users_email"));
        assert!(ddl.contains("UNIQUE (email)"));

        // Test check constraint
        let mut check_params = HashMap::new();
        check_params.insert("condition".to_string(), "age >= 0 AND age <= 150".to_string());
        let check_constraint = Constraint {
            name: "chk_users_age".to_string(),
            table: "users".to_string(),
            constraint_type: ConstraintType::Check,
            columns: vec!["age".to_string()],
            parameters: check_params,
        };
        
        let ddl = generator.generate_constraint_ddl(&check_constraint).unwrap();
        assert!(ddl.contains("ALTER TABLE users"));
        assert!(ddl.contains("ADD CONSTRAINT chk_users_age"));
        assert!(ddl.contains("CHECK (age >= 0 AND age <= 150)"));

        // Test foreign key constraint
        let mut fk_params = HashMap::new();
        fk_params.insert("referenced_table".to_string(), "users".to_string());
        fk_params.insert("referenced_column".to_string(), "id".to_string());
        let fk_constraint = Constraint {
            name: "fk_posts_user".to_string(),
            table: "posts".to_string(),
            constraint_type: ConstraintType::ForeignKey,
            columns: vec!["user_id".to_string()],
            parameters: fk_params,
        };
        
        let ddl = generator.generate_constraint_ddl(&fk_constraint).unwrap();
        assert!(ddl.contains("ALTER TABLE posts"));
        assert!(ddl.contains("ADD CONSTRAINT fk_posts_user"));
        assert!(ddl.contains("FOREIGN KEY (user_id)"));
        assert!(ddl.contains("REFERENCES users (id)"));
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
    fn test_generate_complex_schema_ddl() {
        let generator = DDLGenerator::new();
        let schema = create_complex_test_schema();
        
        let ddl = generator.generate_ddl(&schema).unwrap();
        
        // Should generate DDL for both tables
        assert_eq!(ddl.table_statements.len(), 2);
        
        // Should generate foreign key statements
        assert_eq!(ddl.foreign_key_statements.len(), 1);
        assert!(ddl.foreign_key_statements[0].contains("fk_posts_user"));
        
        // Should generate index statements
        assert_eq!(ddl.index_statements.len(), 2);
        
        // Should generate constraint statements
        assert_eq!(ddl.constraint_statements.len(), 2);
        
        // Check complete DDL formatting
        let complete_ddl = ddl.to_string();
        assert!(complete_ddl.contains("CREATE TABLE"));
        assert!(complete_ddl.contains("users"));
        assert!(complete_ddl.contains("posts"));
        assert!(complete_ddl.contains("JSONB"));
        assert!(complete_ddl.contains("TEXT[]"));
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
        assert!(ddl.drop_statements[0].contains("CASCADE"));
    }

    #[test]
    fn test_ddl_with_comments() {
        let config = DDLConfig {
            include_comments: true,
            ..Default::default()
        };
        
        let generator = DDLGenerator::with_config(config);
        let schema = create_test_schema();
        
        let ddl = generator.generate_ddl(&schema).unwrap();
        
        // Should include header comment
        assert!(!ddl.comments.is_empty());
        assert!(ddl.comments[0].contains("PostgreSQL DDL generated"));
        assert!(ddl.comments[0].contains("Generated at:"));
        
        // Table statements should include comments
        assert!(ddl.table_statements[0].contains("-- Table: users"));
    }

    #[test]
    fn test_ddl_summary() {
        let generator = DDLGenerator::new();
        let schema = create_complex_test_schema();
        
        let ddl = generator.generate_ddl(&schema).unwrap();
        let summary = ddl.summary();
        
        assert_eq!(summary.table_count, 2);
        assert_eq!(summary.foreign_key_count, 1);
        assert_eq!(summary.index_count, 2);
        assert_eq!(summary.constraint_count, 2);
        assert_eq!(summary.warning_count, 1);
    }

    #[test]
    fn test_ddl_all_statements_order() {
        let generator = DDLGenerator::new();
        let schema = create_complex_test_schema();
        
        let ddl = generator.generate_ddl(&schema).unwrap();
        let all_statements = ddl.all_statements();
        
        // Should have statements in correct order
        assert!(!all_statements.is_empty());
        
        // Find positions of different statement types
        let table_pos = all_statements.iter().position(|s| s.contains("CREATE TABLE"));
        let fk_pos = all_statements.iter().position(|s| s.contains("FOREIGN KEY"));
        let constraint_pos = all_statements.iter().position(|s| s.contains("ADD CONSTRAINT") && s.contains("UNIQUE"));
        let index_pos = all_statements.iter().position(|s| s.contains("CREATE INDEX"));
        
        // Tables should come before foreign keys
        if let (Some(table), Some(fk)) = (table_pos, fk_pos) {
            assert!(table < fk, "Tables should be created before foreign keys");
        }
        
        // Foreign keys should come before indexes
        if let (Some(fk), Some(idx)) = (fk_pos, index_pos) {
            assert!(fk < idx, "Foreign keys should be created before indexes");
        }
    }

    #[test]
    fn test_format_default_values() {
        let generator = DDLGenerator::new();
        
        // Test string default
        let string_default = generator.format_default_value(&serde_json::json!("test")).unwrap();
        assert_eq!(string_default, "'test'");
        
        // Test string with quotes
        let quoted_string = generator.format_default_value(&serde_json::json!("test's value")).unwrap();
        assert_eq!(quoted_string, "'test''s value'");
        
        // Test number default
        let number_default = generator.format_default_value(&serde_json::json!(42)).unwrap();
        assert_eq!(number_default, "42");
        
        // Test boolean defaults
        let bool_true = generator.format_default_value(&serde_json::json!(true)).unwrap();
        assert_eq!(bool_true, "TRUE");
        
        let bool_false = generator.format_default_value(&serde_json::json!(false)).unwrap();
        assert_eq!(bool_false, "FALSE");
        
        // Test null default
        let null_default = generator.format_default_value(&serde_json::json!(null)).unwrap();
        assert_eq!(null_default, "NULL");
    }

    #[test]
    fn test_schema_name_formatting() {
        let config = DDLConfig {
            schema_name: Some("custom_schema".to_string()),
            ..Default::default()
        };
        
        let generator = DDLGenerator::with_config(config);
        
        // Test table name formatting
        let formatted = generator.format_table_name("test_table");
        assert_eq!(formatted, "custom_schema.test_table");
        
        // Test without schema name
        let generator_no_schema = DDLGenerator::new();
        let formatted_no_schema = generator_no_schema.format_table_name("test_table");
        assert_eq!(formatted_no_schema, "test_table");
    }

    #[test]
    fn test_drop_statements_generation() {
        let config = DDLConfig {
            include_drop_statements: true,
            ..Default::default()
        };
        
        let generator = DDLGenerator::with_config(config);
        let schema = create_complex_test_schema();
        
        let ddl = generator.generate_ddl(&schema).unwrap();
        
        assert_eq!(ddl.drop_statements.len(), 2);
        
        // Should drop in reverse order (posts before users due to foreign key)
        assert!(ddl.drop_statements[0].contains("DROP TABLE IF EXISTS posts CASCADE"));
        assert!(ddl.drop_statements[1].contains("DROP TABLE IF EXISTS users CASCADE"));
    }

    #[test]
    fn test_error_handling_invalid_constraint() {
        let generator = DDLGenerator::new();
        
        // Test check constraint without condition parameter
        let invalid_constraint = Constraint {
            name: "invalid_check".to_string(),
            table: "test".to_string(),
            constraint_type: ConstraintType::Check,
            columns: vec!["col".to_string()],
            parameters: HashMap::new(), // Missing condition parameter
        };
        
        let result = generator.generate_constraint_ddl(&invalid_constraint);
        assert!(result.is_err());
        
        // Test foreign key constraint without required parameters
        let invalid_fk = Constraint {
            name: "invalid_fk".to_string(),
            table: "test".to_string(),
            constraint_type: ConstraintType::ForeignKey,
            columns: vec!["col".to_string()],
            parameters: HashMap::new(), // Missing referenced_table and referenced_column
        };
        
        let result = generator.generate_constraint_ddl(&invalid_fk);
        assert!(result.is_err());
    }
}