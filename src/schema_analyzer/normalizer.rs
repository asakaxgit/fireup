use crate::error::FireupResult;
use crate::types::{
    SchemaAnalysis, NormalizedSchema, TableDefinition, ColumnDefinition,
    PostgreSQLType, ForeignKeyDefinition, IndexDefinition, Relationship, SchemaWarning, WarningLevel, SchemaMetadata,
    RelationshipType, NormalizationType, NormalizationOpportunity, PrimaryKeyDefinition,
    postgres_identifier,
    KEY_ID_COLUMN, KEY_KIND_COLUMN, KEY_NAME_COLUMN, KEY_NAMESPACE_COLUMN, KEY_PATH_COLUMN,
    PARENT_KEY_PATH_COLUMN,
};
use std::collections::{BTreeSet, HashSet};
use tracing::{info, debug};
use chrono::Utc;
use uuid::Uuid;

/// Normalization engine that applies 1NF, 2NF, and 3NF rules
pub struct NormalizationEngine {
    /// Whether to apply aggressive normalization
    aggressive_normalization: bool,
    /// Minimum threshold for creating separate tables
    separation_threshold: f64,
}

impl NormalizationEngine {
    /// Create a new normalization engine
    pub fn new() -> Self {
        Self {
            aggressive_normalization: false,
            separation_threshold: 0.3, // 30% occurrence rate
        }
    }

    /// Create a new normalization engine with aggressive settings
    pub fn new_aggressive() -> Self {
        Self {
            aggressive_normalization: true,
            separation_threshold: 0.1, // 10% occurrence rate
        }
    }

    /// Normalize schema based on analysis results
    pub fn normalize_schema(&self, analysis: &SchemaAnalysis) -> FireupResult<NormalizedSchema> {
        info!("Starting schema normalization for {} collections", analysis.collections.len());
        
        let mut normalized_schema = NormalizedSchema {
            tables: Vec::new(),
            relationships: Vec::new(),
            constraints: Vec::new(),
            warnings: Vec::new(),
            metadata: SchemaMetadata {
                generated_at: Utc::now(),
                source_analysis_id: Uuid::new_v4(),
                version: "1.0.0".to_string(),
                table_count: 0,
                relationship_count: 0,
            },
        };

        // Apply First Normal Form (1NF) - eliminate repeating groups
        self.apply_first_normal_form(analysis, &mut normalized_schema)?;
        
        // Apply Second Normal Form (2NF) - eliminate partial dependencies
        self.apply_second_normal_form(analysis, &mut normalized_schema)?;
        
        // Apply Third Normal Form (3NF) - eliminate transitive dependencies
        self.apply_third_normal_form(analysis, &mut normalized_schema)?;
        
        // Generate indexes for performance
        self.generate_recommended_indexes(&mut normalized_schema)?;
        
        // Update metadata
        normalized_schema.metadata.table_count = normalized_schema.tables.len() as u32;
        normalized_schema.metadata.relationship_count = normalized_schema.relationships.len() as u32;
        
        info!("Schema normalization completed with {} tables and {} relationships", 
               normalized_schema.tables.len(), normalized_schema.relationships.len());
        
        Ok(normalized_schema)
    }

    /// Apply First Normal Form - eliminate repeating groups by creating separate tables for arrays
    fn apply_first_normal_form(&self, analysis: &SchemaAnalysis, schema: &mut NormalizedSchema) -> FireupResult<()> {
        debug!("Applying First Normal Form (1NF)");
        
        for collection in &analysis.collections {
            let mut main_table = TableDefinition::new(postgres_identifier(&collection.name));
            Self::add_key_columns(&mut main_table, &collection.name);

            // Process field types for this collection. Sorted so the generated
            // DDL is stable across runs — `field_types` comes out of a HashMap.
            let field_prefix = format!("{}.", collection.name);
            let mut collection_fields: Vec<_> = analysis.field_types.iter()
                .filter(|ft| ft.field_path.starts_with(&field_prefix))
                .collect();
            collection_fields.sort_by(|a, b| a.field_path.cmp(&b.field_path));

            let field_names: HashSet<&str> = collection_fields.iter()
                .map(|ft| &ft.field_path[field_prefix.len()..])
                .collect();

            let mut array_tables = Vec::new();
            let mut used_column_names: HashSet<String> = main_table.columns
                .iter()
                .map(|column| column.name.clone())
                .collect();
            let mut maps_kept_whole: BTreeSet<&str> = BTreeSet::new();

            for field_type in collection_fields {
                let field_name = &field_type.field_path[field_prefix.len()..];

                // A nested map is emitted as a single JSONB column, so its leaves
                // must not also become flattened columns — emitting both doubles
                // storage and lets the two copies drift apart.
                if let Some(ancestor) = Self::emitted_ancestor(field_name, &field_names) {
                    maps_kept_whole.insert(ancestor);
                    continue;
                }

                // Check if this field represents an array that should be normalized
                if self.should_normalize_array(field_type, &analysis.normalization_opportunities) {
                    // Create separate table for array elements, keyed on the
                    // parent entity's natural key plus the element's position.
                    let array_table_name =
                        postgres_identifier(&format!("{}_{}", collection.name, field_name));
                    let mut array_table = TableDefinition::new(array_table_name.clone());

                    array_table.add_column(ColumnDefinition::new(
                        PARENT_KEY_PATH_COLUMN.to_string(),
                        PostgreSQLType::Text,
                    ).not_null());
                    array_table.add_column(ColumnDefinition::new("array_index".to_string(), PostgreSQLType::Integer).not_null());
                    array_table.add_column(ColumnDefinition::new("value".to_string(), field_type.recommended_type.clone()));

                    array_table.set_primary_key(PrimaryKeyDefinition {
                        name: postgres_identifier(&format!("{}_pkey", array_table_name)),
                        columns: vec![PARENT_KEY_PATH_COLUMN.to_string(), "array_index".to_string()],
                    });

                    // Add foreign key relationship
                    array_table.add_foreign_key(ForeignKeyDefinition {
                        column: PARENT_KEY_PATH_COLUMN.to_string(),
                        referenced_table: collection.name.clone(),
                        referenced_column: KEY_PATH_COLUMN.to_string(),
                        constraint_name: postgres_identifier(&format!(
                            "fk_{}_{}",
                            array_table_name, collection.name
                        )),
                    });

                    array_tables.push(array_table);

                    // Add relationship
                    schema.relationships.push(Relationship {
                        from_table: array_table_name,
                        to_table: collection.name.clone(),
                        from_column: PARENT_KEY_PATH_COLUMN.to_string(),
                        to_column: KEY_PATH_COLUMN.to_string(),
                        relationship_type: RelationshipType::ManyToOne,
                    });
                } else {
                    // Add as regular column to main table
                    let column_name = Self::unique_column_name(
                        field_name.replace('.', "_"), // Handle nested fields
                        &mut used_column_names,
                    );
                    // Firestore/Datastore schemas are sparse: a field missing (or
                    // explicitly null) in even one document cannot be NOT NULL.
                    let nullable = field_type.presence_percentage < 100.0;

                    // The column name is derived, so record the field it came
                    // from; the importer reads values back out by field path.
                    let mut column =
                        ColumnDefinition::new(column_name, field_type.recommended_type.clone())
                            .from_field(field_name);
                    if !nullable {
                        column = column.not_null();
                    }

                    main_table.add_column(column);
                }
            }

            for map_field in maps_kept_whole {
                schema.warnings.push(SchemaWarning {
                    level: WarningLevel::Info,
                    message: format!(
                        "Nested map '{}' is stored as a single JSONB column; its sub-fields were not flattened into separate columns",
                        map_field
                    ),
                    context: format!("{}.{}", collection.name, map_field),
                    suggestion: Some(
                        "Query the JSONB column, or add an explicit projection if flat columns are required".to_string(),
                    ),
                });
            }

            schema.tables.push(main_table);
            schema.tables.extend(array_tables);
        }

        Ok(())
    }

    /// Add the Datastore key columns and the natural primary key to a table.
    ///
    /// `key_path` is the namespace-qualified key path (`Users/ada`,
    /// `[tenant]Users/ada/Orders/1001`), which makes it unique across
    /// namespaces and ancestor paths. Keying rows on it rather than on a fresh
    /// UUID is what makes a re-import idempotent and the rows joinable.
    fn add_key_columns(table: &mut TableDefinition, collection: &str) {
        table.add_column(
            ColumnDefinition::new(KEY_PATH_COLUMN.to_string(), PostgreSQLType::Text).not_null(),
        );
        table.add_column(
            ColumnDefinition::new(KEY_NAMESPACE_COLUMN.to_string(), PostgreSQLType::Text)
                .not_null()
                .with_default(serde_json::json!("")),
        );
        table.add_column(
            ColumnDefinition::new(KEY_KIND_COLUMN.to_string(), PostgreSQLType::Text).not_null(),
        );
        // Exactly one of these is populated: Datastore keys carry either an
        // application-assigned name or an auto-allocated int64 id.
        table.add_column(ColumnDefinition::new(
            KEY_NAME_COLUMN.to_string(),
            PostgreSQLType::Text,
        ));
        table.add_column(ColumnDefinition::new(
            KEY_ID_COLUMN.to_string(),
            PostgreSQLType::BigInt,
        ));
        // NULL for a root entity; the parent's key_path for a child entity.
        table.add_column(ColumnDefinition::new(
            PARENT_KEY_PATH_COLUMN.to_string(),
            PostgreSQLType::Text,
        ));

        table.set_primary_key(PrimaryKeyDefinition {
            name: postgres_identifier(&format!("{}_pkey", collection)),
            columns: vec![KEY_PATH_COLUMN.to_string()],
        });
    }

    /// Nearest ancestor field of `field_name` that is itself emitted as a column.
    fn emitted_ancestor<'a>(field_name: &str, field_names: &HashSet<&'a str>) -> Option<&'a str> {
        let mut boundary = field_name.rfind('.');
        while let Some(index) = boundary {
            let ancestor = &field_name[..index];
            if let Some(found) = field_names.get(ancestor) {
                return Some(found);
            }
            boundary = ancestor.rfind('.');
        }
        None
    }

    /// Reserve a column name, shortened to fit PostgreSQL and suffixed if the
    /// table already uses it.
    ///
    /// The shortening happens first: PostgreSQL truncates a long identifier
    /// silently, so without it two long property names can collide *after* this
    /// function has declared them unique.
    fn unique_column_name(candidate: String, used: &mut HashSet<String>) -> String {
        let candidate = postgres_identifier(&candidate);
        if used.insert(candidate.clone()) {
            return candidate;
        }
        let mut suffix = 1;
        loop {
            let alternative = postgres_identifier(&format!("{}_{}", candidate, suffix));
            if used.insert(alternative.clone()) {
                return alternative;
            }
            suffix += 1;
        }
    }

    /// Check if an array field should be normalized into a separate table
    fn should_normalize_array(&self, field_type: &crate::types::FieldTypeAnalysis, opportunities: &[NormalizationOpportunity]) -> bool {
        // Check if there's a 1NF opportunity for this field
        opportunities.iter().any(|opp| {
            opp.field_path == field_type.field_path && 
            matches!(opp.normalization_type, NormalizationType::FirstNormalForm)
        }) && field_type.presence_percentage >= (self.separation_threshold * 100.0)
    }

    /// Apply Second Normal Form - eliminate partial dependencies
    fn apply_second_normal_form(&self, analysis: &SchemaAnalysis, schema: &mut NormalizedSchema) -> FireupResult<()> {
        debug!("Applying Second Normal Form (2NF)");
        
        // Look for composite key dependencies
        for opportunity in &analysis.normalization_opportunities {
            if matches!(opportunity.normalization_type, NormalizationType::SecondNormalForm) {
                // For now, add a warning that manual review is needed
                schema.warnings.push(SchemaWarning {
                    level: WarningLevel::Warning,
                    message: format!("Potential 2NF violation in collection '{}': {}", 
                                   opportunity.collection, opportunity.description),
                    context: opportunity.field_path.clone(),
                    suggestion: Some("Review for partial dependencies and consider extracting to separate table".to_string()),
                });
            }
        }
        
        Ok(())
    }

    /// Apply Third Normal Form - eliminate transitive dependencies
    fn apply_third_normal_form(&self, analysis: &SchemaAnalysis, schema: &mut NormalizedSchema) -> FireupResult<()> {
        debug!("Applying Third Normal Form (3NF)");
        
        // Extract transitive dependencies based on detected relationships
        for relationship in &analysis.relationships {
            if relationship.confidence > 0.8 {
                // Create lookup table for high-confidence relationships
                self.create_lookup_table(relationship, schema)?;
            }
        }
        
        // Look for transitive dependency opportunities
        for opportunity in &analysis.normalization_opportunities {
            if matches!(opportunity.normalization_type, NormalizationType::ThirdNormalForm) {
                schema.warnings.push(SchemaWarning {
                    level: WarningLevel::Info,
                    message: format!("Potential 3NF optimization in collection '{}': {}", 
                                   opportunity.collection, opportunity.description),
                    context: opportunity.field_path.clone(),
                    suggestion: Some("Consider extracting transitive dependencies to reference tables".to_string()),
                });
            }
        }
        
        Ok(())
    }

    /// Create a lookup table for a detected relationship
    fn create_lookup_table(&self, relationship: &crate::types::DetectedRelationship, schema: &mut NormalizedSchema) -> FireupResult<()> {
        let lookup_table_name = format!("{}_{}_lookup", relationship.from_collection, relationship.to_collection);

        // Check if table already exists
        if schema.tables.iter().any(|t| t.name == lookup_table_name) {
            return Ok(());
        }

        // A foreign key to a table this export does not contain would make the
        // whole DDL script unloadable, so only link kinds we actually generated.
        let has_table = |name: &str| schema.tables.iter().any(|t| t.name == name);
        if !has_table(&relationship.from_collection) || !has_table(&relationship.to_collection) {
            schema.warnings.push(SchemaWarning {
                level: WarningLevel::Warning,
                message: format!(
                    "Skipped lookup table for '{}' → '{}': the referenced kind is not present in this export",
                    relationship.from_collection, relationship.to_collection
                ),
                context: relationship.reference_field.clone(),
                suggestion: Some("Export both kinds together to generate the relationship".to_string()),
            });
            return Ok(());
        }

        let from_column = format!("{}_{}", relationship.from_collection, KEY_PATH_COLUMN);
        let to_column = format!("{}_{}", relationship.to_collection, KEY_PATH_COLUMN);

        let mut lookup_table = TableDefinition::new(postgres_identifier(&lookup_table_name));

        // Both sides reference the natural key of the entity table, so the pair
        // itself identifies the row — no surrogate id is needed.
        lookup_table.add_column(
            ColumnDefinition::new(from_column.clone(), PostgreSQLType::Text).not_null(),
        );
        lookup_table.add_column(
            ColumnDefinition::new(to_column.clone(), PostgreSQLType::Text).not_null(),
        );

        lookup_table.set_primary_key(PrimaryKeyDefinition {
            name: postgres_identifier(&format!("{}_pkey", lookup_table_name)),
            columns: vec![from_column.clone(), to_column.clone()],
        });

        // Add foreign keys
        lookup_table.add_foreign_key(ForeignKeyDefinition {
            column: from_column.clone(),
            referenced_table: relationship.from_collection.clone(),
            referenced_column: KEY_PATH_COLUMN.to_string(),
            constraint_name: postgres_identifier(&format!(
                "fk_{}_{}",
                lookup_table_name, relationship.from_collection
            )),
        });

        lookup_table.add_foreign_key(ForeignKeyDefinition {
            column: to_column.clone(),
            referenced_table: relationship.to_collection.clone(),
            referenced_column: KEY_PATH_COLUMN.to_string(),
            constraint_name: postgres_identifier(&format!(
                "fk_{}_{}",
                lookup_table_name, relationship.to_collection
            )),
        });

        schema.tables.push(lookup_table);

        // Add relationships
        schema.relationships.push(Relationship {
            from_table: lookup_table_name.clone(),
            to_table: relationship.from_collection.clone(),
            from_column,
            to_column: KEY_PATH_COLUMN.to_string(),
            relationship_type: RelationshipType::ManyToOne,
        });

        schema.relationships.push(Relationship {
            from_table: lookup_table_name,
            to_table: relationship.to_collection.clone(),
            from_column: to_column,
            to_column: KEY_PATH_COLUMN.to_string(),
            relationship_type: RelationshipType::ManyToOne,
        });

        Ok(())
    }

    /// Generate recommended indexes for performance
    fn generate_recommended_indexes(&self, schema: &mut NormalizedSchema) -> FireupResult<()> {
        debug!("Generating recommended indexes");
        
        for table in &mut schema.tables {
            // Add indexes for foreign key columns
            let foreign_keys = table.foreign_keys.clone(); // Clone to avoid borrow checker issues
            for fk in &foreign_keys {
                let index_name = format!("idx_{}_{}", table.name, fk.column);
                table.add_index(IndexDefinition {
                    name: index_name,
                    columns: vec![fk.column.clone()],
                    unique: false,
                    index_type: Some("btree".to_string()),
                });
            }
            
            // No index is added for the primary key: PostgreSQL already creates
            // a unique btree index to back the PRIMARY KEY constraint, so a
            // second one would only duplicate storage and write cost.
        }
        
        Ok(())
    }
}

impl Default for NormalizationEngine {
    fn default() -> Self {
        Self::new()
    }
}