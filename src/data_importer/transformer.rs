use crate::error::FireupError;
use crate::types::{FirestoreDocument, NormalizedSchema, TableDefinition, ColumnDefinition, PostgreSQLType};
use crate::data_importer::type_mapper::DataTypeMapper;
use serde_json::{Value, Map};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Result of document transformation process
#[derive(Debug, Clone)]
pub struct TransformationResult {
    /// Generated SQL INSERT statements
    pub sql_statements: Vec<String>,
    /// Transformed data organized by table
    pub table_data: HashMap<String, Vec<TableRow>>,
    /// Warnings encountered during transformation
    pub warnings: Vec<String>,
    /// Statistics about the transformation
    pub statistics: TransformationStatistics,
}

/// A single row of data for a table
#[derive(Debug, Clone)]
pub struct TableRow {
    /// Column values for this row
    pub columns: HashMap<String, Value>,
    /// Primary key value for this row
    pub primary_key: Value,
    /// Foreign key relationships
    pub foreign_keys: HashMap<String, Value>,
}

/// Statistics about the transformation process
#[derive(Debug, Clone)]
pub struct TransformationStatistics {
    /// Total number of documents processed
    pub documents_processed: u64,
    /// Number of tables created
    pub tables_created: u32,
    /// Total number of rows generated
    pub total_rows: u64,
    /// Number of normalization operations performed
    pub normalizations_performed: u32,
    /// Processing time in milliseconds
    pub processing_time_ms: u64,
}

/// Configuration for document transformation
#[derive(Debug, Clone)]
pub struct TransformationConfig {
    /// Whether to generate UUIDs for missing primary keys
    pub generate_missing_ids: bool,
    /// Maximum depth for nested object flattening
    pub max_nesting_depth: u32,
    /// Whether to preserve original document structure in JSONB
    pub preserve_original: bool,
    /// Batch size for processing large collections
    pub batch_size: usize,
}

impl Default for TransformationConfig {
    fn default() -> Self {
        Self {
            generate_missing_ids: true,
            max_nesting_depth: 5,
            preserve_original: false,
            batch_size: 1000,
        }
    }
}

/// Transforms Firestore documents into relational table data
pub struct DocumentTransformer {
    /// Type mapper for converting Firestore types to PostgreSQL
    type_mapper: DataTypeMapper,
    /// Configuration for transformation behavior
    config: TransformationConfig,
    /// Cache for generated UUIDs to maintain consistency
    id_cache: HashMap<String, Uuid>,
}

impl DocumentTransformer {
    /// Create a new document transformer with default configuration
    pub fn new() -> Self {
        Self {
            type_mapper: DataTypeMapper::new(),
            config: TransformationConfig::default(),
            id_cache: HashMap::new(),
        }
    }

    /// Create a new document transformer with custom configuration
    pub fn with_config(config: TransformationConfig) -> Self {
        Self {
            type_mapper: DataTypeMapper::new(),
            config,
            id_cache: HashMap::new(),
        }
    }

    /// Create a new document transformer with custom type mapper and configuration
    pub fn with_type_mapper_and_config(type_mapper: DataTypeMapper, config: TransformationConfig) -> Self {
        Self {
            type_mapper,
            config,
            id_cache: HashMap::new(),
        }
    }

    /// Transform documents according to a normalized schema
    pub fn transform_documents(
        &mut self, 
        documents: &[FirestoreDocument], 
        schema: &NormalizedSchema
    ) -> Result<TransformationResult, FireupError> {
        let start_time = std::time::Instant::now();
        let mut table_data: HashMap<String, Vec<TableRow>> = HashMap::new();
        let mut warnings = Vec::new();
        let mut statistics = TransformationStatistics {
            documents_processed: 0,
            tables_created: schema.tables.len() as u32,
            total_rows: 0,
            normalizations_performed: 0,
            processing_time_ms: 0,
        };

        // Initialize table data structures
        for table in &schema.tables {
            table_data.insert(table.name.clone(), Vec::new());
        }

        // Process documents in batches
        for batch in documents.chunks(self.config.batch_size) {
            for document in batch {
                match self.transform_single_document(document, schema, &mut table_data) {
                    Ok(doc_warnings) => {
                        warnings.extend(doc_warnings);
                        statistics.documents_processed += 1;
                    }
                    Err(e) => {
                        warnings.push(format!("Failed to transform document {}: {}", document.id, e));
                    }
                }
            }
        }

        // Calculate total rows
        statistics.total_rows = table_data.values().map(|rows| rows.len() as u64).sum();
        statistics.processing_time_ms = start_time.elapsed().as_millis() as u64;

        // Generate SQL statements
        let sql_statements = self.generate_sql_statements(&table_data, schema)?;

        Ok(TransformationResult {
            sql_statements,
            table_data,
            warnings,
            statistics,
        })
    }

    /// Transform a single Firestore document
    fn transform_single_document(
        &mut self,
        document: &FirestoreDocument,
        schema: &NormalizedSchema,
        table_data: &mut HashMap<String, Vec<TableRow>>,
    ) -> Result<Vec<String>, FireupError> {
        let mut warnings = Vec::new();

        // Find the main table for this document's collection
        let main_table = schema.tables.iter()
            .find(|t| t.name == document.collection || t.name == format!("{}_main", document.collection))
            .ok_or_else(|| FireupError::TypeMapping(
                format!("No table found for collection: {}", document.collection)
            ))?;

        // Transform the main document
        let main_row = self.transform_document_to_row(document, main_table, &mut warnings)?;
        
        if let Some(rows) = table_data.get_mut(&main_table.name) {
            rows.push(main_row);
        }

        // Handle nested structures and arrays that require normalization
        self.transform_nested_structures(document, schema, table_data, &mut warnings)?;

        // Handle subcollections
        for subcollection in &document.subcollections {
            let sub_warnings = self.transform_single_document(subcollection, schema, table_data)?;
            warnings.extend(sub_warnings);
        }

        Ok(warnings)
    }

    /// Transform a document into a table row
    fn transform_document_to_row(
        &mut self,
        document: &FirestoreDocument,
        table: &TableDefinition,
        warnings: &mut Vec<String>,
    ) -> Result<TableRow, FireupError> {
        let mut columns = HashMap::new();
        let mut foreign_keys = HashMap::new();

        // Generate or use existing primary key
        let primary_key = if document.data.contains_key("id") {
            document.data["id"].clone()
        } else if self.config.generate_missing_ids {
            let uuid = self.get_or_generate_uuid(&document.id);
            Value::String(uuid.to_string())
        } else {
            Value::String(document.id.clone())
        };

        // Add primary key to columns
        if let Some(ref pk) = table.primary_key {
            if let Some(pk_column) = pk.columns.first() {
                columns.insert(pk_column.clone(), primary_key.clone());
            }
        }

        // Transform each field according to the table schema
        for column in &table.columns {
            if let Some(ref pk) = table.primary_key {
                if pk.columns.contains(&column.name) {
                    continue; // Already handled primary key
                }
            }

            let field_value = self.extract_field_value(&document.data, &column.name, &column.column_type)?;
            
            match field_value {
                Some(value) => {
                    // Check if this is a foreign key
                    if let Some(fk) = table.foreign_keys.iter().find(|fk| fk.column == column.name) {
                        let fk_value = self.transform_foreign_key_value(&value, &fk.referenced_table)?;
                        foreign_keys.insert(column.name.clone(), fk_value.clone());
                        columns.insert(column.name.clone(), fk_value);
                    } else {
                        let transformed_value = self.transform_value_for_column(&value, column, warnings)?;
                        columns.insert(column.name.clone(), transformed_value);
                    }
                }
                None => {
                    if !column.nullable {
                        warnings.push(format!(
                            "Required field '{}' is missing in document '{}'", 
                            column.name, document.id
                        ));
                    }
                    // Use default value or NULL
                    if let Some(default) = &column.default_value {
                        columns.insert(column.name.clone(), default.clone());
                    } else if column.nullable {
                        columns.insert(column.name.clone(), Value::Null);
                    }
                }
            }
        }

        Ok(TableRow {
            columns,
            primary_key,
            foreign_keys,
        })
    }

    /// Extract field value from document data, handling nested paths
    fn extract_field_value(
        &self,
        data: &HashMap<String, Value>,
        field_path: &str,
        expected_type: &PostgreSQLType,
    ) -> Result<Option<Value>, FireupError> {
        let path_parts: Vec<&str> = field_path.split('.').collect();
        
        if path_parts.len() == 1 {
            // Simple field access
            Ok(data.get(field_path).cloned())
        } else {
            // Nested field access
            let mut current_value = data.get(path_parts[0]);
            
            for part in &path_parts[1..] {
                match current_value {
                    Some(Value::Object(obj)) => {
                        current_value = obj.get(*part);
                    }
                    Some(Value::Array(arr)) => {
                        // Handle array index access
                        if let Ok(index) = part.parse::<usize>() {
                            current_value = arr.get(index);
                        } else {
                            return Ok(None);
                        }
                    }
                    _ => return Ok(None),
                }
            }
            
            Ok(current_value.cloned())
        }
    }

    /// Transform a value to match the expected column type
    fn transform_value_for_column(
        &self,
        value: &Value,
        column: &ColumnDefinition,
        warnings: &mut Vec<String>,
    ) -> Result<Value, FireupError> {
        match (&column.column_type, value) {
            // Direct type matches
            (PostgreSQLType::Boolean, Value::Bool(_)) => Ok(value.clone()),
            (PostgreSQLType::Integer, Value::Number(n)) if n.is_i64() => Ok(value.clone()),
            (PostgreSQLType::BigInt, Value::Number(n)) if n.is_i64() => Ok(value.clone()),
            (PostgreSQLType::Numeric(_, _), Value::Number(_)) => Ok(value.clone()),
            (PostgreSQLType::Text, Value::String(_)) => Ok(value.clone()),
            (PostgreSQLType::Varchar(_), Value::String(_)) => Ok(value.clone()),
            (PostgreSQLType::Jsonb, _) => Ok(value.clone()),
            
            // Type conversions
            (PostgreSQLType::Uuid, Value::String(s)) => {
                if let Ok(uuid) = Uuid::parse_str(s) {
                    Ok(Value::String(uuid.to_string()))
                } else {
                    warnings.push(format!("Invalid UUID format: {}", s));
                    Ok(Value::String(Uuid::new_v4().to_string()))
                }
            }
            
            (PostgreSQLType::Timestamp, Value::String(s)) => {
                // Try to parse timestamp string
                if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                    Ok(Value::String(dt.to_rfc3339()))
                } else {
                    warnings.push(format!("Invalid timestamp format: {}", s));
                    Ok(value.clone())
                }
            }
            
            // String to number conversions
            (PostgreSQLType::Integer, Value::String(s)) => {
                if let Ok(num) = s.parse::<i32>() {
                    Ok(Value::Number(serde_json::Number::from(num)))
                } else {
                    warnings.push(format!("Cannot convert string '{}' to integer", s));
                    Ok(Value::Null)
                }
            }
            
            // Array handling
            (PostgreSQLType::Array(_), Value::Array(_)) => Ok(value.clone()),
            
            // Fallback to JSONB for complex types
            _ => {
                warnings.push(format!(
                    "Type mismatch for column '{}': expected {:?}, got {:?}. Using JSONB.",
                    column.name, column.column_type, value
                ));
                Ok(value.clone())
            }
        }
    }

    /// Transform foreign key value to appropriate format
    pub fn transform_foreign_key_value(
        &mut self,
        value: &Value,
        referenced_table: &str,
    ) -> Result<Value, FireupError> {
        match value {
            Value::String(s) => {
                // Check if it's a Firestore reference path
                if s.contains("/documents/") {
                    // Extract document ID from reference path
                    let parts: Vec<&str> = s.split('/').collect();
                    if let Some(doc_id) = parts.last() {
                        let uuid = self.get_or_generate_uuid(doc_id);
                        Ok(Value::String(uuid.to_string()))
                    } else {
                        Err(FireupError::TypeMapping(format!("Invalid reference format: {}", s)))
                    }
                } else if Uuid::parse_str(s).is_ok() {
                    // Already a UUID
                    Ok(value.clone())
                } else {
                    // Generate UUID for string ID
                    let uuid = self.get_or_generate_uuid(s);
                    Ok(Value::String(uuid.to_string()))
                }
            }
            _ => {
                // Convert other types to string and generate UUID
                let string_val = value.to_string();
                let uuid = self.get_or_generate_uuid(&string_val);
                Ok(Value::String(uuid.to_string()))
            }
        }
    }

    /// Handle nested structures that require normalization
    fn transform_nested_structures(
        &mut self,
        document: &FirestoreDocument,
        schema: &NormalizedSchema,
        table_data: &mut HashMap<String, Vec<TableRow>>,
        warnings: &mut Vec<String>,
    ) -> Result<(), FireupError> {
        // Find tables that are normalized from this document's collection
        let normalized_tables: Vec<&TableDefinition> = schema.tables.iter()
            .filter(|t| t.name.starts_with(&format!("{}_", document.collection)) && 
                       t.name != document.collection)
            .collect();

        for table in normalized_tables {
            // Extract the field name from the table name
            let field_name = table.name.strip_prefix(&format!("{}_", document.collection))
                .unwrap_or(&table.name);

            if let Some(field_value) = document.data.get(field_name) {
                match field_value {
                    Value::Array(arr) => {
                        self.transform_array_to_normalized_table(
                            arr, table, &document.id, table_data, warnings
                        )?;
                    }
                    Value::Object(obj) => {
                        self.transform_object_to_normalized_table(
                            obj, table, &document.id, table_data, warnings
                        )?;
                    }
                    _ => {
                        warnings.push(format!(
                            "Expected array or object for normalized field '{}', got {:?}",
                            field_name, field_value
                        ));
                    }
                }
            }
        }

        Ok(())
    }

    /// Transform an array into a normalized table
    fn transform_array_to_normalized_table(
        &mut self,
        array: &[Value],
        table: &TableDefinition,
        parent_id: &str,
        table_data: &mut HashMap<String, Vec<TableRow>>,
        warnings: &mut Vec<String>,
    ) -> Result<(), FireupError> {
        let parent_uuid = self.get_or_generate_uuid(parent_id);

        for (index, item) in array.iter().enumerate() {
            let mut columns = HashMap::new();
            let mut foreign_keys = HashMap::new();

            // Generate primary key for this array item
            let item_id = format!("{}_{}", parent_id, index);
            let item_uuid = self.get_or_generate_uuid(&item_id);
            
            if let Some(ref pk) = table.primary_key {
                if let Some(pk_column) = pk.columns.first() {
                    columns.insert(pk_column.clone(), Value::String(item_uuid.to_string()));
                }
            }

            // Add parent foreign key
            if let Some(parent_fk) = table.foreign_keys.iter().find(|fk| fk.referenced_column == "id") {
                foreign_keys.insert(parent_fk.column.clone(), Value::String(parent_uuid.to_string()));
                columns.insert(parent_fk.column.clone(), Value::String(parent_uuid.to_string()));
            }

            // Transform the array item based on its type
            match item {
                Value::Object(obj) => {
                    // Object array item - map each field to table columns
                    for column in &table.columns {
                        let is_primary_key = table.primary_key.as_ref()
                            .map(|pk| pk.columns.contains(&column.name))
                            .unwrap_or(false);
                        if is_primary_key || table.foreign_keys.iter().any(|fk| fk.column == column.name) {
                            continue; // Already handled
                        }

                        if let Some(field_value) = obj.get(&column.name) {
                            let transformed_value = self.transform_value_for_column(field_value, column, warnings)?;
                            columns.insert(column.name.clone(), transformed_value);
                        }
                    }
                }
                _ => {
                    // Primitive array item - store in a 'value' column
                    if let Some(value_column) = table.columns.iter().find(|c| c.name == "value") {
                        let transformed_value = self.transform_value_for_column(item, value_column, warnings)?;
                        columns.insert("value".to_string(), transformed_value);
                    }
                }
            }

            let row = TableRow {
                columns,
                primary_key: Value::String(item_uuid.to_string()),
                foreign_keys,
            };

            if let Some(rows) = table_data.get_mut(&table.name) {
                rows.push(row);
            }
        }

        Ok(())
    }

    /// Transform an object into a normalized table
    fn transform_object_to_normalized_table(
        &mut self,
        object: &Map<String, Value>,
        table: &TableDefinition,
        parent_id: &str,
        table_data: &mut HashMap<String, Vec<TableRow>>,
        warnings: &mut Vec<String>,
    ) -> Result<(), FireupError> {
        let parent_uuid = self.get_or_generate_uuid(parent_id);
        let mut columns = HashMap::new();
        let mut foreign_keys = HashMap::new();

        // Generate primary key for this object
        let object_uuid = self.get_or_generate_uuid(&format!("{}_obj", parent_id));
        
        if let Some(ref pk) = table.primary_key {
            if let Some(pk_column) = pk.columns.first() {
                columns.insert(pk_column.clone(), Value::String(object_uuid.to_string()));
            }
        }

        // Add parent foreign key
        if let Some(parent_fk) = table.foreign_keys.iter().find(|fk| fk.referenced_column == "id") {
            foreign_keys.insert(parent_fk.column.clone(), Value::String(parent_uuid.to_string()));
            columns.insert(parent_fk.column.clone(), Value::String(parent_uuid.to_string()));
        }

        // Map object fields to table columns
        for column in &table.columns {
            let is_primary_key = table.primary_key.as_ref()
                .map(|pk| pk.columns.contains(&column.name))
                .unwrap_or(false);
            if is_primary_key || table.foreign_keys.iter().any(|fk| fk.column == column.name) {
                continue; // Already handled
            }

            if let Some(field_value) = object.get(&column.name) {
                let transformed_value = self.transform_value_for_column(field_value, column, warnings)?;
                columns.insert(column.name.clone(), transformed_value);
            }
        }

        let row = TableRow {
            columns,
            primary_key: Value::String(object_uuid.to_string()),
            foreign_keys,
        };

        if let Some(rows) = table_data.get_mut(&table.name) {
            rows.push(row);
        }

        Ok(())
    }

    /// Generate SQL INSERT statements from table data
    fn generate_sql_statements(
        &self,
        table_data: &HashMap<String, Vec<TableRow>>,
        schema: &NormalizedSchema,
    ) -> Result<Vec<String>, FireupError> {
        let mut statements = Vec::new();

        // Generate statements in dependency order (tables with no foreign keys first)
        let mut processed_tables = HashSet::new();
        let mut remaining_tables: Vec<&TableDefinition> = schema.tables.iter().collect();

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
                            let statement = self.generate_insert_statement(table, rows);
                            statements.push(statement);
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

    /// Generate a single INSERT statement for a table
    fn generate_insert_statement(&self, table: &TableDefinition, rows: &[TableRow]) -> String {
        if rows.is_empty() {
            return format!("-- No data for table {}", table.name);
        }

        let column_names: Vec<String> = table.columns.iter().map(|c| c.name.clone()).collect();
        let columns_clause = column_names.join(", ");

        let mut values_clauses = Vec::new();
        for row in rows {
            let mut values = Vec::new();
            for column_name in &column_names {
                if let Some(value) = row.columns.get(column_name) {
                    values.push(self.format_sql_value(value));
                } else {
                    values.push("NULL".to_string());
                }
            }
            values_clauses.push(format!("({})", values.join(", ")));
        }

        format!(
            "INSERT INTO {} ({}) VALUES\n{};",
            table.name,
            columns_clause,
            values_clauses.join(",\n")
        )
    }

    /// Format a JSON value for SQL insertion
    fn format_sql_value(&self, value: &Value) -> String {
        match value {
            Value::Null => "NULL".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            Value::String(s) => format!("'{}'", s.replace('\'', "''")), // Escape single quotes
            Value::Array(_) | Value::Object(_) => {
                format!("'{}'", value.to_string().replace('\'', "''"))
            }
        }
    }

    /// Get or generate a consistent UUID for a given ID
    fn get_or_generate_uuid(&mut self, id: &str) -> Uuid {
        if let Some(uuid) = self.id_cache.get(id) {
            *uuid
        } else {
            let uuid = Uuid::new_v4();
            self.id_cache.insert(id.to_string(), uuid);
            uuid
        }
    }
}

impl Default for DocumentTransformer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use serde_json::json;

    fn create_test_schema() -> NormalizedSchema {
        let mut schema = NormalizedSchema {
            tables: vec![],
            relationships: vec![],
            constraints: vec![],
            warnings: vec![],
            metadata: SchemaMetadata {
                generated_at: Utc::now(),
                source_analysis_id: Uuid::new_v4(),
                version: "1.0".to_string(),
                table_count: 0,
                relationship_count: 0,
            },
        };

        // Create a simple users table
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
        schema.metadata.table_count = 1;

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

        doc
    }

    #[test]
    fn test_transform_simple_document() {
        let mut transformer = DocumentTransformer::new();
        let schema = create_test_schema();
        let document = create_test_document();

        let result = transformer.transform_documents(&[document], &schema).unwrap();

        assert_eq!(result.statistics.documents_processed, 1);
        assert_eq!(result.table_data.len(), 1);
        assert!(result.table_data.contains_key("users"));

        let users_rows = &result.table_data["users"];
        assert_eq!(users_rows.len(), 1);

        let row = &users_rows[0];
        assert!(row.columns.contains_key("name"));
        assert_eq!(row.columns["name"], json!("John Doe"));
    }

    #[test]
    fn test_generate_sql_statements() {
        let mut transformer = DocumentTransformer::new();
        let schema = create_test_schema();
        let document = create_test_document();

        let result = transformer.transform_documents(&[document], &schema).unwrap();

        assert!(!result.sql_statements.is_empty());
        let sql = &result.sql_statements[0];
        assert!(sql.contains("INSERT INTO users"));
        assert!(sql.contains("John Doe"));
    }

    #[test]
    fn test_foreign_key_transformation() {
        let mut transformer = DocumentTransformer::new();
        
        // Test Firestore reference transformation
        let reference = "projects/test/databases/(default)/documents/users/user123";
        let result = transformer.transform_foreign_key_value(
            &json!(reference), 
            "users"
        ).unwrap();

        if let Value::String(uuid_str) = result {
            assert!(Uuid::parse_str(&uuid_str).is_ok());
        } else {
            panic!("Expected UUID string");
        }
    }

    #[test]
    fn test_type_conversion_warnings() {
        let mut transformer = DocumentTransformer::new();
        let mut warnings = Vec::new();

        let column = ColumnDefinition::new("age".to_string(), PostgreSQLType::Integer);
        let result = transformer.transform_value_for_column(
            &json!("not_a_number"), 
            &column, 
            &mut warnings
        ).unwrap();

        assert!(!warnings.is_empty());
        assert_eq!(result, Value::Null);
    }
}