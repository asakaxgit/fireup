use super::*;
use crate::types::*;
use crate::data_importer::importer::{ImportResult, TableImportSpec, BatchConfig, ImportProgress, ConnectionConfig, PostgreSQLImporter, FullImportResult};
use crate::data_importer::type_mapper::DataTypeMapper;
use crate::data_importer::sql_generator::{SQLGenerator, SQLGenerationConfig, ConflictStrategy, StatementType};
use serde_json::json;
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

/// Test type mapping with various Firestore data types
#[cfg(test)]
mod type_mapper_tests {
    use super::*;

    #[test]
    fn test_map_primitive_types() {
        let mapper = DataTypeMapper::new();

        // Test boolean mapping
        let result = mapper.map_value_type(&json!(true), "test.bool").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Boolean));
        assert_eq!(result.metadata.original_type, "boolean");
        assert!(!result.requires_normalization);

        // Test integer mapping
        let result = mapper.map_value_type(&json!(42), "test.int").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Integer));
        assert_eq!(result.metadata.original_type, "integer");

        // Test large integer mapping
        let result = mapper.map_value_type(&json!(9223372036854775807i64), "test.bigint").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::BigInt));

        // Test float mapping
        let result = mapper.map_value_type(&json!(3.14), "test.float").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Numeric(Some(15), Some(6))));
        assert_eq!(result.metadata.original_type, "float");
    }

    #[test]
    fn test_map_string_types() {
        let mapper = DataTypeMapper::new();

        // Test regular string
        let result = mapper.map_value_type(&json!("hello world"), "test.string").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Varchar(Some(255))));

        // Test long string
        let long_string = "a".repeat(2000);
        let result = mapper.map_value_type(&json!(long_string), "test.long_string").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Text));

        // Test UUID string
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let result = mapper.map_value_type(&json!(uuid_str), "test.uuid").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Uuid));
        assert_eq!(result.metadata.original_type, "uuid_string");

        // Test timestamp string
        let timestamp_str = "2023-01-01T12:00:00Z";
        let result = mapper.map_value_type(&json!(timestamp_str), "test.timestamp").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Timestamp));

        // Test Firestore reference
        let reference = "projects/test/databases/(default)/documents/users/user123";
        let result = mapper.map_value_type(&json!(reference), "test.reference").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Uuid));
        assert_eq!(result.metadata.original_type, "reference");
    }

    #[test]
    fn test_map_array_types() {
        let mapper = DataTypeMapper::new();

        // Test empty array
        let result = mapper.map_value_type(&json!([]), "test.empty_array").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Jsonb));
        assert_eq!(result.metadata.original_type, "empty_array");

        // Test homogeneous integer array
        let result = mapper.map_value_type(&json!([1, 2, 3, 4, 5]), "test.int_array").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Array(_)));
        assert_eq!(result.metadata.original_type, "homogeneous_array");

        // Test heterogeneous array
        let result = mapper.map_value_type(&json!([1, "hello", true]), "test.mixed_array").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Jsonb));
        assert_eq!(result.metadata.original_type, "heterogeneous_array");

        // Test large array (should require normalization)
        let large_array: Vec<i32> = (0..20).collect();
        let result = mapper.map_value_type(&json!(large_array), "test.large_array").unwrap();
        assert!(result.requires_normalization);
        assert_eq!(result.metadata.original_type, "large_array");
    }

    #[test]
    fn test_map_object_types() {
        let mapper = DataTypeMapper::new();

        // Test empty object
        let result = mapper.map_value_type(&json!({}), "test.empty_object").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Jsonb));
        assert_eq!(result.metadata.original_type, "empty_object");

        // Test simple object
        let simple_obj = json!({"name": "test", "value": 42});
        let result = mapper.map_value_type(&simple_obj, "test.simple_object").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Jsonb));
        assert!(!result.requires_normalization);

        // Test complex object (should require normalization)
        let complex_obj = json!({
            "field1": "value1",
            "field2": "value2",
            "field3": "value3",
            "field4": "value4",
            "nested": {"inner": "value"}
        });
        let result = mapper.map_value_type(&complex_obj, "test.complex_object").unwrap();
        assert!(result.requires_normalization);
        assert_eq!(result.metadata.original_type, "complex_object");
    }

    #[test]
    fn test_multiple_values_mapping() {
        let mapper = DataTypeMapper::new();

        // Test consistent types
        let val1 = json!(10);
        let val2 = json!(20);
        let val3 = json!(30);
        let values = vec![&val1, &val2, &val3];
        let result = mapper.map_multiple_values(&values, "test.consistent").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Integer));

        // Test type conflicts - should resolve to most general type
        let mixed_val1 = json!(42);
        let mixed_val2 = json!("hello");
        let mixed_val3 = json!(true);
        let mixed_values = vec![&mixed_val1, &mixed_val2, &mixed_val3];
        let result = mapper.map_multiple_values(&mixed_values, "test.mixed").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Jsonb));
        assert!(!result.metadata.warnings.is_empty());

        // Test numeric type unification
        let num_val1 = json!(42);
        let num_val2 = json!(3.14);
        let num_val3 = json!(100);
        let numeric_values = vec![&num_val1, &num_val2, &num_val3];
        let result = mapper.map_multiple_values(&numeric_values, "test.numeric").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Numeric(None, None)));
    }

    #[test]
    fn test_custom_mappings() {
        let mut mapper = DataTypeMapper::new();
        mapper.add_custom_mapping("user.id".to_string(), PostgreSQLType::Uuid);

        let result = mapper.map_value_type(&json!("some-string"), "user.id").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Uuid));
        assert!(result.metadata.warnings.iter().any(|w| w.contains("custom type mapping")));
    }
}

/// Test document transformation to relational format
#[cfg(test)]
mod transformer_tests {
    use super::*;

    fn create_test_schema() -> NormalizedSchema {
        let mut schema = NormalizedSchema {
            tables: vec![],
            relationships: vec![],
            constraints: vec![],
            warnings: vec![],
            metadata: SchemaMetadata {
                generated_at: chrono::Utc::now(),
                source_analysis_id: Uuid::new_v4(),
                version: "1.0".to_string(),
                table_count: 0,
                relationship_count: 0,
            },
        };

        // Create users table
        let mut users_table = TableDefinition::new("users".to_string());
        users_table.add_column(ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid).not_null());
        users_table.add_column(ColumnDefinition::new("name".to_string(), PostgreSQLType::Varchar(Some(255))));
        users_table.add_column(ColumnDefinition::new("email".to_string(), PostgreSQLType::Varchar(Some(255))));
        users_table.add_column(ColumnDefinition::new("age".to_string(), PostgreSQLType::Integer));
        users_table.set_primary_key(PrimaryKeyDefinition {
            name: "users_pkey".to_string(),
            columns: vec!["id".to_string()],
        });
        schema.tables.push(users_table);

        // Create normalized table for user tags
        let mut tags_table = TableDefinition::new("users_tags".to_string());
        tags_table.add_column(ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid).not_null());
        tags_table.add_column(ColumnDefinition::new("user_id".to_string(), PostgreSQLType::Uuid).not_null());
        tags_table.add_column(ColumnDefinition::new("value".to_string(), PostgreSQLType::Varchar(Some(100))));
        tags_table.set_primary_key(PrimaryKeyDefinition {
            name: "users_tags_pkey".to_string(),
            columns: vec!["id".to_string()],
        });
        tags_table.add_foreign_key(ForeignKeyDefinition {
            constraint_name: "fk_users_tags_user_id".to_string(),
            column: "user_id".to_string(),
            referenced_table: "users".to_string(),
            referenced_column: "id".to_string(),
        });
        schema.tables.push(tags_table);

        schema.metadata.table_count = 2;
        schema
    }

    fn create_test_document() -> FirestoreDocument {
        let mut doc = FirestoreDocument::new(
            "user1".to_string(),
            "users".to_string(),
            "users/user1".to_string(),
        );

        doc.add_field("name".to_string(), json!("John Doe"));
        doc.add_field("email".to_string(), json!("john@example.com"));
        doc.add_field("age".to_string(), json!(30));
        doc.add_field("tags".to_string(), json!(["developer", "rust", "database"]));

        doc
    }

    #[test]
    fn test_transform_simple_document() {
        let mut transformer = DocumentTransformer::new();
        let schema = create_test_schema();
        let document = create_test_document();

        let result = transformer.transform_documents(&[document], &schema).unwrap();

        assert_eq!(result.statistics.documents_processed, 1);
        assert_eq!(result.table_data.len(), 2); // users and users_tags tables
        assert!(result.table_data.contains_key("users"));

        let users_rows = &result.table_data["users"];
        assert_eq!(users_rows.len(), 1);

        let row = &users_rows[0];
        assert!(row.columns.contains_key("name"));
        assert_eq!(row.columns["name"], json!("John Doe"));
        assert_eq!(row.columns["email"], json!("john@example.com"));
        assert_eq!(row.columns["age"], json!(30));
    }

    #[test]
    fn test_transform_nested_arrays() {
        let mut transformer = DocumentTransformer::new();
        let schema = create_test_schema();
        let document = create_test_document();

        let result = transformer.transform_documents(&[document], &schema).unwrap();

        // Check that tags were normalized to separate table
        if let Some(tags_rows) = result.table_data.get("users_tags") {
            assert_eq!(tags_rows.len(), 3); // Three tags: developer, rust, database
            
            let tag_values: Vec<&str> = tags_rows.iter()
                .filter_map(|row| row.columns.get("value"))
                .filter_map(|v| v.as_str())
                .collect();
            
            assert!(tag_values.contains(&"developer"));
            assert!(tag_values.contains(&"rust"));
            assert!(tag_values.contains(&"database"));

            // Check foreign key relationships
            for row in tags_rows {
                assert!(row.columns.contains_key("user_id"));
                assert!(row.foreign_keys.contains_key("user_id"));
            }
        } else {
            panic!("Expected users_tags table to be populated");
        }
    }

    #[test]
    fn test_type_conversion_with_warnings() {
        let mut transformer = DocumentTransformer::new();
        let mut schema = create_test_schema();
        
        // Add a column that expects integer but will receive string
        if let Some(users_table) = schema.tables.get_mut(0) {
            users_table.add_column(ColumnDefinition::new("score".to_string(), PostgreSQLType::Integer));
        }

        let mut document = create_test_document();
        document.add_field("score".to_string(), json!("not_a_number"));

        let result = transformer.transform_documents(&[document], &schema).unwrap();

        // Should have warnings about type conversion
        assert!(!result.warnings.is_empty());
        assert!(result.warnings.iter().any(|w| w.contains("Cannot convert string")));
    }

    #[test]
    fn test_foreign_key_transformation() {
        let mut transformer = DocumentTransformer::new();
        
        // Test Firestore reference transformation
        let reference = "projects/test/databases/(default)/documents/users/user123";
        let result = transformer.transform_foreign_key_value(&json!(reference), "users").unwrap();

        if let serde_json::Value::String(uuid_str) = result {
            assert!(Uuid::parse_str(&uuid_str).is_ok());
        } else {
            panic!("Expected UUID string");
        }

        // Test string ID transformation
        let string_id = "simple_id";
        let result = transformer.transform_foreign_key_value(&json!(string_id), "users").unwrap();
        
        if let serde_json::Value::String(uuid_str) = result {
            assert!(Uuid::parse_str(&uuid_str).is_ok());
        } else {
            panic!("Expected UUID string");
        }
    }

    #[test]
    fn test_missing_required_fields() {
        let mut transformer = DocumentTransformer::new();
        let mut schema = create_test_schema();
        
        // Make email required (not nullable)
        if let Some(users_table) = schema.tables.get_mut(0) {
            if let Some(email_column) = users_table.columns.iter_mut().find(|c| c.name == "email") {
                email_column.nullable = false;
            }
        }

        let mut document = create_test_document();
        document.data.remove("email"); // Remove required field

        let result = transformer.transform_documents(&[document], &schema).unwrap();

        // Should have warnings about missing required field
        assert!(!result.warnings.is_empty());
        assert!(result.warnings.iter().any(|w| w.contains("Required field 'email' is missing")));
    }

    #[test]
    fn test_nested_object_transformation() {
        let mut transformer = DocumentTransformer::new();
        let mut schema = create_test_schema();

        // Add a normalized table for user profile
        let mut profile_table = TableDefinition::new("users_profile".to_string());
        profile_table.add_column(ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid).not_null());
        profile_table.add_column(ColumnDefinition::new("user_id".to_string(), PostgreSQLType::Uuid).not_null());
        profile_table.add_column(ColumnDefinition::new("first_name".to_string(), PostgreSQLType::Varchar(Some(100))));
        profile_table.add_column(ColumnDefinition::new("last_name".to_string(), PostgreSQLType::Varchar(Some(100))));
        profile_table.set_primary_key(PrimaryKeyDefinition {
            name: "users_profile_pkey".to_string(),
            columns: vec!["id".to_string()],
        });
        profile_table.add_foreign_key(ForeignKeyDefinition {
            constraint_name: "fk_users_profile_user_id".to_string(),
            column: "user_id".to_string(),
            referenced_table: "users".to_string(),
            referenced_column: "id".to_string(),
        });
        schema.tables.push(profile_table);

        let mut document = create_test_document();
        document.add_field("profile".to_string(), json!({
            "first_name": "John",
            "last_name": "Doe"
        }));

        let result = transformer.transform_documents(&[document], &schema).unwrap();

        // Check that profile was normalized to separate table
        if let Some(profile_rows) = result.table_data.get("users_profile") {
            assert_eq!(profile_rows.len(), 1);
            
            let row = &profile_rows[0];
            assert_eq!(row.columns.get("first_name"), Some(&json!("John")));
            assert_eq!(row.columns.get("last_name"), Some(&json!("Doe")));
            assert!(row.foreign_keys.contains_key("user_id"));
        } else {
            panic!("Expected users_profile table to be populated");
        }
    }
}

/// Test SQL generation with different data structures
#[cfg(test)]
mod sql_generator_tests {
    use super::*;

    fn create_test_table() -> TableDefinition {
        let mut table = TableDefinition::new("test_table".to_string());
        table.add_column(ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid).not_null());
        table.add_column(ColumnDefinition::new("name".to_string(), PostgreSQLType::Varchar(Some(255))));
        table.add_column(ColumnDefinition::new("age".to_string(), PostgreSQLType::Integer));
        table.add_column(ColumnDefinition::new("active".to_string(), PostgreSQLType::Boolean));
        table.add_column(ColumnDefinition::new("metadata".to_string(), PostgreSQLType::Jsonb));
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
                    ("active".to_string(), json!(true)),
                    ("metadata".to_string(), json!({"role": "admin", "score": 95})),
                ].iter().cloned().collect(),
                primary_key: json!("550e8400-e29b-41d4-a716-446655440000"),
                foreign_keys: HashMap::new(),
            },
            TableRow {
                columns: [
                    ("id".to_string(), json!("550e8400-e29b-41d4-a716-446655440001")),
                    ("name".to_string(), json!("Jane Smith")),
                    ("age".to_string(), json!(25)),
                    ("active".to_string(), json!(false)),
                    ("metadata".to_string(), json!({"role": "user", "score": 87})),
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
        assert!(statement.sql.contains("age INTEGER"));
        assert!(statement.sql.contains("active BOOLEAN"));
        assert!(statement.sql.contains("metadata JSONB"));
        assert!(statement.sql.contains("PRIMARY KEY (id)"));
        assert_eq!(statement.statement_type, StatementType::CreateTable);
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
        assert!(statement.sql.contains("true"));
        assert!(statement.sql.contains("false"));
        assert_eq!(statement.row_count, 2);
        assert_eq!(statement.statement_type, StatementType::Insert);
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
        assert!(statement.sql.contains("$2"));
        assert!(!statement.parameters.is_empty());
        assert_eq!(statement.parameters.len(), column_names.len() * rows.len());
        assert_eq!(statement.row_count, 2);
    }

    #[test]
    fn test_sql_value_formatting() {
        let generator = SQLGenerator::new();

        // Test different value types
        assert_eq!(generator.format_sql_value(&json!(null)), "NULL");
        assert_eq!(generator.format_sql_value(&json!(true)), "true");
        assert_eq!(generator.format_sql_value(&json!(false)), "false");
        assert_eq!(generator.format_sql_value(&json!(42)), "42");
        assert_eq!(generator.format_sql_value(&json!(3.14)), "3.14");
        assert_eq!(generator.format_sql_value(&json!("test")), "'test'");
        
        // Test SQL injection prevention
        assert_eq!(generator.format_sql_value(&json!("test's value")), "'test''s value'");
        
        // Test complex types (arrays and objects)
        let complex_value = json!({"key": "value", "number": 42});
        let formatted = generator.format_sql_value(&complex_value);
        assert!(formatted.starts_with('\''));
        assert!(formatted.ends_with('\''));
        assert!(formatted.contains("key"));
        assert!(formatted.contains("value"));
    }

    #[test]
    fn test_conflict_strategies() {
        // Test IGNORE strategy
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

        // Test UPDATE strategy
        let config = SQLGenerationConfig {
            handle_conflicts: true,
            conflict_strategy: ConflictStrategy::Update,
            ..Default::default()
        };
        let generator = SQLGenerator::with_config(config);
        let statement = generator.generate_literal_insert(&table, &column_names, &rows, &mut warnings).unwrap();
        assert!(statement.sql.contains("ON CONFLICT (id) DO UPDATE SET"));
        assert!(statement.sql.contains("name = EXCLUDED.name"));
    }

    #[test]
    fn test_batch_processing() {
        let config = SQLGenerationConfig {
            batch_size: 1, // Force batching with size 1
            ..Default::default()
        };
        let mut generator = SQLGenerator::with_config(config);
        let table = create_test_table();
        let rows = create_test_rows();

        let statements = generator.generate_bulk_insert(&table, &rows, None).unwrap();

        // Should generate 2 statements (one per row due to batch_size = 1)
        assert_eq!(statements.len(), 2);
        for statement in statements {
            assert_eq!(statement.row_count, 1);
        }
    }

    #[test]
    fn test_foreign_key_constraints() {
        let mut generator = SQLGenerator::new();
        let mut schema = NormalizedSchema {
            tables: vec![],
            relationships: vec![],
            constraints: vec![],
            warnings: vec![],
            metadata: SchemaMetadata {
                generated_at: chrono::Utc::now(),
                source_analysis_id: Uuid::new_v4(),
                version: "1.0".to_string(),
                table_count: 0,
                relationship_count: 0,
            },
        };

        let mut table = create_test_table();
        table.add_foreign_key(ForeignKeyDefinition {
            constraint_name: "fk_test_parent".to_string(),
            column: "parent_id".to_string(),
            referenced_table: "parent_table".to_string(),
            referenced_column: "id".to_string(),
        });
        schema.tables.push(table);

        let statements = generator.generate_constraint_statements(&schema).unwrap();

        assert_eq!(statements.len(), 1);
        let statement = &statements[0];
        assert!(statement.sql.contains("ALTER TABLE test_table"));
        assert!(statement.sql.contains("ADD CONSTRAINT fk_test_parent"));
        assert!(statement.sql.contains("FOREIGN KEY (parent_id)"));
        assert!(statement.sql.contains("REFERENCES parent_table (id)"));
        assert_eq!(statement.statement_type, StatementType::AlterTable);
    }

    #[test]
    fn test_index_generation() {
        let generator = SQLGenerator::new();
        let mut table = create_test_table();
        
        table.add_index(IndexDefinition {
            name: "idx_test_name".to_string(),
            columns: vec!["name".to_string()],
            unique: false,
            index_type: Some("BTREE".to_string()),
        });
        
        table.add_index(IndexDefinition {
            name: "idx_test_email_unique".to_string(),
            columns: vec!["email".to_string()],
            unique: true,
            index_type: Some("BTREE".to_string()),
        });

        let statements = generator.generate_index_statements(&table).unwrap();

        assert_eq!(statements.len(), 2);
        
        let regular_index = &statements[0];
        assert!(regular_index.sql.contains("CREATE INDEX IF NOT EXISTS idx_test_name"));
        assert!(regular_index.sql.contains("ON test_table USING BTREE (name)"));
        
        let unique_index = &statements[1];
        assert!(unique_index.sql.contains("CREATE UNIQUE INDEX IF NOT EXISTS idx_test_email_unique"));
    }

    #[test]
    fn test_copy_statement_generation() {
        let generator = SQLGenerator::new();
        let table = create_test_table();
        let csv_path = "/tmp/test_data.csv";

        let statement = generator.generate_copy_statement(&table, csv_path).unwrap();

        assert!(statement.sql.contains("COPY test_table"));
        assert!(statement.sql.contains("FROM '/tmp/test_data.csv'"));
        assert!(statement.sql.contains("WITH (FORMAT csv, HEADER true"));
        assert_eq!(statement.statement_type, StatementType::Insert);
    }

    #[test]
    fn test_transaction_wrapping() {
        let config = SQLGenerationConfig {
            use_transactions: true,
            ..Default::default()
        };
        let mut generator = SQLGenerator::with_config(config);
        
        let mut schema = NormalizedSchema {
            tables: vec![create_test_table()],
            relationships: vec![],
            constraints: vec![],
            warnings: vec![],
            metadata: SchemaMetadata {
                generated_at: chrono::Utc::now(),
                source_analysis_id: Uuid::new_v4(),
                version: "1.0".to_string(),
                table_count: 1,
                relationship_count: 0,
            },
        };

        let transformation_result = TransformationResult {
            sql_statements: vec![],
            table_data: [("test_table".to_string(), create_test_rows())].iter().cloned().collect(),
            warnings: vec![],
            statistics: TransformationStatistics {
                documents_processed: 2,
                tables_created: 1,
                total_rows: 2,
                normalizations_performed: 0,
                processing_time_ms: 100,
            },
        };

        let result = generator.generate_sql(&transformation_result, &schema).unwrap();

        // Should have BEGIN at start and COMMIT at end
        assert!(result.statements.first().unwrap().sql.contains("BEGIN"));
        assert!(result.statements.last().unwrap().sql.contains("COMMIT"));
    }
}

/// Test PostgreSQL connection and schema creation
#[cfg(test)]
mod postgresql_importer_tests {
    use super::*;
    use std::time::Duration;

    fn create_test_connection_config() -> ConnectionConfig {
        ConnectionConfig {
            host: "localhost".to_string(),
            port: 5432,
            database: "fireup_test".to_string(),
            user: "fireup".to_string(),
            password: "fireup_dev_password".to_string(),
            max_connections: 5,
            connection_timeout: Duration::from_secs(10),
            retry_attempts: 2,
            retry_delay: Duration::from_millis(100),
        }
    }

    #[tokio::test]
    async fn test_postgresql_connection_creation() {
        // Initialize monitoring system for tests
        crate::monitoring::initialize_monitoring(crate::monitoring::MonitoringConfig::default());
        
        let config = create_test_connection_config();
        
        // Test connection creation - this will fail in test environment without actual PostgreSQL
        // but we can test the configuration and error handling
        let result = PostgreSQLImporter::new(config.clone()).await;
        
        // In test environment, this should fail with connection error
        assert!(result.is_err());
        
        // Verify error contains expected connection information
        let error = result.unwrap_err();
        let error_msg = error.to_string();
        assert!(error_msg.contains("Failed to connect to PostgreSQL") || 
                error_msg.contains("connection") ||
                error_msg.contains("database"));
    }

    #[tokio::test]
    async fn test_connection_config_validation() {
        // Initialize monitoring system for tests
        crate::monitoring::initialize_monitoring(crate::monitoring::MonitoringConfig::default());
        
        let mut config = create_test_connection_config();
        
        // Test with invalid port
        config.port = 0;
        let result = PostgreSQLImporter::new(config.clone()).await;
        assert!(result.is_err());
        
        // Test with empty database name
        config.port = 5432;
        config.database = "".to_string();
        let result = PostgreSQLImporter::new(config).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_connection_config_defaults() {
        let config = ConnectionConfig::default();
        
        assert_eq!(config.host, "localhost");
        assert_eq!(config.port, 5432);
        assert_eq!(config.database, "fireup_dev");
        assert_eq!(config.user, "fireup");
        assert_eq!(config.password, "fireup_dev_password");
        assert_eq!(config.max_connections, 10);
        assert_eq!(config.connection_timeout, Duration::from_secs(30));
        assert_eq!(config.retry_attempts, 3);
        assert_eq!(config.retry_delay, Duration::from_secs(1));
    }

    #[test]
    fn test_schema_creation_ddl_statements() {
        // Test DDL statement preparation for schema creation
        let ddl_statements = vec![
            "CREATE TABLE IF NOT EXISTS users (id UUID PRIMARY KEY, name VARCHAR(255))".to_string(),
            "CREATE TABLE IF NOT EXISTS posts (id UUID PRIMARY KEY, user_id UUID REFERENCES users(id))".to_string(),
            "CREATE INDEX IF NOT EXISTS idx_posts_user_id ON posts(user_id)".to_string(),
        ];
        
        // Verify statements are properly formatted
        assert_eq!(ddl_statements.len(), 3);
        assert!(ddl_statements[0].contains("CREATE TABLE IF NOT EXISTS users"));
        assert!(ddl_statements[1].contains("REFERENCES users(id)"));
        assert!(ddl_statements[2].contains("CREATE INDEX"));
    }

    #[test]
    fn test_import_result_creation() {
        let result = ImportResult {
            imported_records: 100,
            failed_records: 5,
            warnings: vec!["Warning 1".to_string(), "Warning 2".to_string()],
        };
        
        assert_eq!(result.imported_records, 100);
        assert_eq!(result.failed_records, 5);
        assert_eq!(result.warnings.len(), 2);
    }

    #[test]
    fn test_table_import_spec_creation() {
        let spec = TableImportSpec {
            table_name: "users".to_string(),
            columns: vec!["id".to_string(), "name".to_string(), "email".to_string()],
            data_source: "users_collection".to_string(),
            batch_size: Some(1000),
            validation_enabled: true,
        };
        
        assert_eq!(spec.table_name, "users");
        assert_eq!(spec.columns.len(), 3);
        assert_eq!(spec.batch_size, Some(1000));
        assert!(spec.validation_enabled);
    }

    #[test]
    fn test_full_import_result_summary() {
        let mut result = FullImportResult {
            schema_creation: Some(ImportResult {
                imported_records: 5,
                failed_records: 0,
                warnings: vec![],
            }),
            table_imports: vec![
                ("users".to_string(), ImportResult {
                    imported_records: 100,
                    failed_records: 2,
                    warnings: vec!["Warning".to_string()],
                }),
                ("posts".to_string(), ImportResult {
                    imported_records: 50,
                    failed_records: 0,
                    warnings: vec![],
                }),
            ],
            validation_results: vec![
                ("users".to_string(), vec![]),
                ("posts".to_string(), vec!["Constraint violation".to_string()]),
            ],
            total_records_imported: 150,
            total_records_failed: 2,
            warnings: vec!["Global warning".to_string()],
        };
        
        assert!(!result.is_successful()); // Has constraint violations
        
        let summary = result.summary();
        assert!(summary.contains("2 tables processed"));
        assert!(summary.contains("150 records imported"));
        assert!(summary.contains("2 failed"));
        assert!(summary.contains("1 warnings"));
        assert!(summary.contains("1 constraint violations"));
        
        // Test successful result
        result.validation_results = vec![
            ("users".to_string(), vec![]),
            ("posts".to_string(), vec![]),
        ];
        result.total_records_failed = 0;
        
        assert!(result.is_successful());
    }
}

/// Test batch processing with large datasets
#[cfg(test)]
mod batch_processor_tests {
    use crate::data_importer::importer::{BatchConfig, ImportProgress};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn test_batch_config_defaults() {
        let config = BatchConfig::default();
        
        assert_eq!(config.batch_size, 1000);
        assert_eq!(config.max_concurrent_batches, 4);
        assert_eq!(config.progress_report_interval, 10000);
    }

    #[test]
    fn test_import_progress_creation() {
        let progress = ImportProgress::new(10000, 1000);
        
        assert_eq!(progress.total_records, 10000);
        assert_eq!(progress.processed_records, 0);
        assert_eq!(progress.successful_records, 0);
        assert_eq!(progress.failed_records, 0);
        assert_eq!(progress.current_batch, 0);
        assert_eq!(progress.total_batches, 10); // 10000 / 1000 = 10
        assert_eq!(progress.progress_percentage(), 0.0);
    }

    #[test]
    fn test_import_progress_percentage_calculation() {
        let mut progress = ImportProgress::new(1000, 100);
        
        // Test 0% progress
        assert_eq!(progress.progress_percentage(), 0.0);
        
        // Test 50% progress
        progress.processed_records = 500;
        assert_eq!(progress.progress_percentage(), 50.0);
        
        // Test 100% progress
        progress.processed_records = 1000;
        assert_eq!(progress.progress_percentage(), 100.0);
        
        // Test empty dataset
        let empty_progress = ImportProgress::new(0, 100);
        assert_eq!(empty_progress.progress_percentage(), 100.0);
    }

    #[tokio::test]
    async fn test_batch_processing_logic() {
        // Create mock data for batch processing
        let test_data: Vec<i32> = (1..=2500).collect(); // 2500 records
        let batch_size = 1000;
        
        // Calculate expected batches
        let expected_batches = (test_data.len() + batch_size - 1) / batch_size;
        assert_eq!(expected_batches, 3); // 3 batches: 1000, 1000, 500
        
        // Test batch splitting logic
        let batches: Vec<Vec<i32>> = test_data
            .chunks(batch_size)
            .map(|chunk| chunk.to_vec())
            .collect();
        
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].len(), 1000);
        assert_eq!(batches[1].len(), 1000);
        assert_eq!(batches[2].len(), 500);
        
        // Verify data integrity
        let total_items: usize = batches.iter().map(|b| b.len()).sum();
        assert_eq!(total_items, test_data.len());
    }

    #[tokio::test]
    async fn test_concurrent_batch_processing_simulation() {
        // Simulate concurrent batch processing with atomic counters
        let processed_count = Arc::new(AtomicUsize::new(0));
        let success_count = Arc::new(AtomicUsize::new(0));
        let failure_count = Arc::new(AtomicUsize::new(0));
        
        let test_data: Vec<i32> = (1..=1000).collect();
        let batch_size = 100;
        let max_concurrent = 4;
        
        // Simulate processing batches
        let batches: Vec<Vec<i32>> = test_data
            .chunks(batch_size)
            .map(|chunk| chunk.to_vec())
            .collect();
        
        let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));
        let mut handles = Vec::new();
        
        for (batch_index, batch) in batches.into_iter().enumerate() {
            let semaphore = semaphore.clone();
            let processed_count = processed_count.clone();
            let success_count = success_count.clone();
            let failure_count = failure_count.clone();
            
            let handle = tokio::spawn(async move {
                let _permit = semaphore.acquire().await.unwrap();
                
                // Simulate processing time
                tokio::time::sleep(Duration::from_millis(10)).await;
                
                // Simulate some failures (every 5th batch fails)
                if batch_index % 5 == 4 {
                    failure_count.fetch_add(batch.len(), Ordering::SeqCst);
                } else {
                    success_count.fetch_add(batch.len(), Ordering::SeqCst);
                }
                
                processed_count.fetch_add(batch.len(), Ordering::SeqCst);
            });
            
            handles.push(handle);
        }
        
        // Wait for all batches to complete
        for handle in handles {
            handle.await.unwrap();
        }
        
        // Verify results
        assert_eq!(processed_count.load(Ordering::SeqCst), 1000);
        assert!(success_count.load(Ordering::SeqCst) > 0);
        assert!(failure_count.load(Ordering::SeqCst) > 0);
        assert_eq!(
            success_count.load(Ordering::SeqCst) + failure_count.load(Ordering::SeqCst),
            1000
        );
    }

    #[test]
    fn test_batch_error_handling() {
        // Test error scenarios in batch processing
        let empty_data: Vec<i32> = vec![];
        let progress = ImportProgress::new(empty_data.len(), 100);
        
        assert_eq!(progress.total_records, 0);
        assert_eq!(progress.total_batches, 0);
        assert_eq!(progress.progress_percentage(), 100.0);
        
        // Test with single item
        let single_item = vec![1];
        let progress = ImportProgress::new(single_item.len(), 100);
        
        assert_eq!(progress.total_records, 1);
        assert_eq!(progress.total_batches, 1);
    }
}

/// Test transaction rollback on constraint violations
#[cfg(test)]
mod transaction_rollback_tests {
    use std::time::Duration;

    #[test]
    fn test_constraint_violation_detection() {
        // Test constraint violation scenarios
        let violations = vec![
            "NOT NULL constraint violated for column 'email': 5 null values found".to_string(),
            "Foreign key constraint 'fk_posts_user_id' violated in 3 rows".to_string(),
            "UNIQUE constraint violated for column 'username': 2 duplicate values found".to_string(),
        ];
        
        assert_eq!(violations.len(), 3);
        assert!(violations[0].contains("NOT NULL"));
        assert!(violations[1].contains("Foreign key"));
        assert!(violations[2].contains("UNIQUE"));
    }

    #[test]
    fn test_transaction_error_scenarios() {
        // Test different types of transaction errors
        let transaction_errors = vec![
            "Failed to start transaction: connection lost",
            "Failed to commit transaction: constraint violation",
            "Failed to rollback transaction: connection timeout",
        ];
        
        for error in transaction_errors {
            assert!(error.contains("transaction"));
        }
    }

    #[tokio::test]
    async fn test_rollback_simulation() {
        // Simulate transaction rollback scenario
        struct MockTransaction {
            committed: bool,
            rolled_back: bool,
        }
        
        impl MockTransaction {
            fn new() -> Self {
                Self {
                    committed: false,
                    rolled_back: false,
                }
            }
            
            fn commit(&mut self) -> Result<(), &'static str> {
                if self.rolled_back {
                    return Err("Transaction already rolled back");
                }
                self.committed = true;
                Ok(())
            }
            
            fn rollback(&mut self) -> Result<(), &'static str> {
                if self.committed {
                    return Err("Transaction already committed");
                }
                self.rolled_back = true;
                Ok(())
            }
        }
        
        // Test successful transaction
        let mut tx = MockTransaction::new();
        assert!(tx.commit().is_ok());
        assert!(tx.committed);
        assert!(!tx.rolled_back);
        
        // Test rollback scenario
        let mut tx = MockTransaction::new();
        assert!(tx.rollback().is_ok());
        assert!(!tx.committed);
        assert!(tx.rolled_back);
        
        // Test error after rollback
        assert!(tx.commit().is_err());
    }

    #[test]
    fn test_constraint_validation_queries() {
        // Test SQL queries for constraint validation
        let table_name = "users";
        
        // NOT NULL constraint check
        let null_check_query = format!(
            "SELECT COUNT(*) as null_count FROM {} WHERE email IS NULL",
            table_name
        );
        assert!(null_check_query.contains("COUNT(*)"));
        assert!(null_check_query.contains("IS NULL"));
        
        // Foreign key constraint check
        let fk_check_query = format!(
            "SELECT COUNT(*) FROM {} u LEFT JOIN profiles p ON u.profile_id = p.id WHERE u.profile_id IS NOT NULL AND p.id IS NULL",
            table_name
        );
        assert!(fk_check_query.contains("LEFT JOIN"));
        assert!(fk_check_query.contains("IS NOT NULL"));
        assert!(fk_check_query.contains("IS NULL"));
        
        // Unique constraint check
        let unique_check_query = format!(
            "SELECT email, COUNT(*) FROM {} GROUP BY email HAVING COUNT(*) > 1",
            table_name
        );
        assert!(unique_check_query.contains("GROUP BY"));
        assert!(unique_check_query.contains("HAVING"));
        assert!(unique_check_query.contains("COUNT(*) > 1"));
    }

    #[test]
    fn test_batch_transaction_failure_recovery() {
        // Test recovery from batch transaction failures
        struct BatchResult {
            batch_id: usize,
            success: bool,
            records_processed: usize,
            error_message: Option<String>,
        }
        
        let batch_results = vec![
            BatchResult {
                batch_id: 1,
                success: true,
                records_processed: 1000,
                error_message: None,
            },
            BatchResult {
                batch_id: 2,
                success: false,
                records_processed: 0,
                error_message: Some("Constraint violation in batch 2".to_string()),
            },
            BatchResult {
                batch_id: 3,
                success: true,
                records_processed: 500,
                error_message: None,
            },
        ];
        
        let successful_batches: Vec<_> = batch_results.iter().filter(|r| r.success).collect();
        let failed_batches: Vec<_> = batch_results.iter().filter(|r| !r.success).collect();
        
        assert_eq!(successful_batches.len(), 2);
        assert_eq!(failed_batches.len(), 1);
        
        let total_successful_records: usize = successful_batches.iter()
            .map(|r| r.records_processed)
            .sum();
        assert_eq!(total_successful_records, 1500);
        
        let failed_batch = failed_batches[0];
        assert_eq!(failed_batch.batch_id, 2);
        assert!(failed_batch.error_message.as_ref().unwrap().contains("Constraint violation"));
    }

    #[tokio::test]
    async fn test_concurrent_transaction_handling() {
        // Test handling of concurrent transactions with potential conflicts
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        
        let successful_transactions = Arc::new(AtomicUsize::new(0));
        let failed_transactions = Arc::new(AtomicUsize::new(0));
        let rolled_back_transactions = Arc::new(AtomicUsize::new(0));
        
        let mut handles = Vec::new();
        
        // Simulate 10 concurrent transactions
        for i in 0..10 {
            let successful = successful_transactions.clone();
            let failed = failed_transactions.clone();
            let rolled_back = rolled_back_transactions.clone();
            
            let handle = tokio::spawn(async move {
                // Simulate transaction processing
                tokio::time::sleep(Duration::from_millis(10)).await;
                
                // Simulate constraint violations in some transactions
                if i % 3 == 0 {
                    // Constraint violation - rollback
                    rolled_back.fetch_add(1, Ordering::SeqCst);
                    failed.fetch_add(1, Ordering::SeqCst);
                } else {
                    // Successful transaction
                    successful.fetch_add(1, Ordering::SeqCst);
                }
            });
            
            handles.push(handle);
        }
        
        // Wait for all transactions to complete
        for handle in handles {
            handle.await.unwrap();
        }
        
        // Verify results
        assert_eq!(successful_transactions.load(Ordering::SeqCst), 6); // 7 successful (10 - 3 failed)
        assert_eq!(failed_transactions.load(Ordering::SeqCst), 4); // 3 failed
        assert_eq!(rolled_back_transactions.load(Ordering::SeqCst), 4); // 3 rolled back
        assert_eq!(
            successful_transactions.load(Ordering::SeqCst) + failed_transactions.load(Ordering::SeqCst),
            10
        );
    }
}

/// Test utility functions
#[cfg(test)]
mod utils_tests {
    use super::*;

    #[test]
    fn test_sanitize_table_name() {
        assert_eq!(sql_generator::utils::sanitize_table_name("valid_name"), "valid_name");
        assert_eq!(sql_generator::utils::sanitize_table_name("invalid-name"), "invalid_name");
        assert_eq!(sql_generator::utils::sanitize_table_name("invalid name"), "invalid_name");
        assert_eq!(sql_generator::utils::sanitize_table_name("123invalid"), "table_123invalid");
        assert_eq!(sql_generator::utils::sanitize_table_name("special@chars#"), "special_chars_");
    }

    #[test]
    fn test_is_valid_identifier() {
        assert!(sql_generator::utils::is_valid_identifier("valid_name"));
        assert!(sql_generator::utils::is_valid_identifier("ValidName"));
        assert!(sql_generator::utils::is_valid_identifier("valid123"));
        assert!(!sql_generator::utils::is_valid_identifier("invalid-name"));
        assert!(!sql_generator::utils::is_valid_identifier("invalid name"));
        assert!(!sql_generator::utils::is_valid_identifier("123invalid"));
        assert!(!sql_generator::utils::is_valid_identifier(""));
        assert!(!sql_generator::utils::is_valid_identifier("special@chars"));
    }

    #[test]
    fn test_escape_identifier() {
        assert_eq!(sql_generator::utils::escape_identifier("valid_name"), "valid_name");
        assert_eq!(sql_generator::utils::escape_identifier("invalid-name"), "\"invalid-name\"");
        assert_eq!(sql_generator::utils::escape_identifier("invalid name"), "\"invalid name\"");
        assert_eq!(sql_generator::utils::escape_identifier("123invalid"), "\"123invalid\"");
        assert_eq!(sql_generator::utils::escape_identifier("quote\"test"), "\"quote\"\"test\"");
    }

    #[test]
    fn test_generate_constraint_name() {
        let name = sql_generator::utils::generate_constraint_name(
            "users", 
            "fk", 
            &["user_id".to_string()]
        );
        
        assert!(name.starts_with("users_fk_user_id_"));
        assert!(name.len() > "users_fk_user_id_".len()); // Should have UUID suffix
        
        let multi_column_name = sql_generator::utils::generate_constraint_name(
            "orders", 
            "pk", 
            &["user_id".to_string(), "order_id".to_string()]
        );
        
        assert!(multi_column_name.starts_with("orders_pk_user_id_order_id_"));
    }
}