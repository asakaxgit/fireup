use crate::error::{FireupError, FireupResult};
use crate::types::{
    SchemaAnalysis, NormalizedSchema, TableDefinition, ColumnDefinition, 
    PostgreSQLType, ForeignKeyDefinition, IndexDefinition, Relationship,
    Constraint, ConstraintType, SchemaWarning, WarningLevel, SchemaMetadata,
    RelationshipType, NormalizationType, NormalizationOpportunity, PrimaryKeyDefinition
};
use std::collections::{HashMap, HashSet};
use serde_json::Value;
use tracing::{info, debug, warn};
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
            let mut main_table = TableDefinition::new(collection.name.clone());
            
            // Add primary key
            main_table.add_column(ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid).not_null());
            main_table.set_primary_key(PrimaryKeyDefinition {
                name: format!("{}_pkey", collection.name),
                columns: vec!["id".to_string()],
            });
            
            // Process field types for this collection
            let collection_fields: Vec<_> = analysis.field_types.iter()
                .filter(|ft| ft.field_path.starts_with(&format!("{}.", collection.name)))
                .collect();
            
            let mut array_tables = Vec::new();
            
            for field_type in collection_fields {
                let field_name = field_type.field_path
                    .strip_prefix(&format!("{}.", collection.name))
                    .unwrap_or(&field_type.field_path);
                
                // Check if this field represents an array that should be normalized
                if self.should_normalize_array(field_type, &analysis.normalization_opportunities) {
                    // Create separate table for array elements
                    let array_table_name = format!("{}_{}", collection.name, field_name);
                    let mut array_table = TableDefinition::new(array_table_name.clone());
                    
                    // Add columns for array table
                    array_table.add_column(ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid).not_null());
                    array_table.add_column(ColumnDefinition::new(
                        format!("{}_id", collection.name), 
                        PostgreSQLType::Uuid
                    ).not_null());
                    array_table.add_column(ColumnDefinition::new("value".to_string(), field_type.recommended_type.clone()));
                    array_table.add_column(ColumnDefinition::new("array_index".to_string(), PostgreSQLType::Integer));
                    
                    array_table.set_primary_key(PrimaryKeyDefinition {
                        name: format!("{}_pkey", array_table_name),
                        columns: vec!["id".to_string()],
                    });
                    
                    // Add foreign key relationship
                    array_table.add_foreign_key(ForeignKeyDefinition {
                        column: format!("{}_id", collection.name),
                        referenced_table: collection.name.clone(),
                        referenced_column: "id".to_string(),
                        constraint_name: format!("fk_{}_{}", array_table_name, collection.name),
                    });
                    
                    array_tables.push(array_table);
                    
                    // Add relationship
                    schema.relationships.push(Relationship {
                        from_table: array_table_name,
                        to_table: collection.name.clone(),
                        from_column: format!("{}_id", collection.name),
                        to_column: "id".to_string(),
                        relationship_type: RelationshipType::ManyToOne,
                    });
                } else {
                    // Add as regular column to main table
                    let column_name = field_name.replace('.', "_"); // Handle nested fields
                    let nullable = field_type.presence_percentage < 95.0; // Not null if present in 95%+ of documents
                    
                    let mut column = ColumnDefinition::new(column_name, field_type.recommended_type.clone());
                    if !nullable {
                        column = column.not_null();
                    }
                    
                    main_table.add_column(column);
                }
            }
            
            schema.tables.push(main_table);
            schema.tables.extend(array_tables);
        }
        
        Ok(())
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
        
        let mut lookup_table = TableDefinition::new(lookup_table_name.clone());
        
        // Add columns
        lookup_table.add_column(ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid).not_null());
        lookup_table.add_column(ColumnDefinition::new(
            format!("{}_id", relationship.from_collection), 
            PostgreSQLType::Uuid
        ).not_null());
        lookup_table.add_column(ColumnDefinition::new(
            format!("{}_id", relationship.to_collection), 
            PostgreSQLType::Uuid
        ).not_null());
        
        lookup_table.set_primary_key(PrimaryKeyDefinition {
            name: format!("{}_pkey", lookup_table_name),
            columns: vec!["id".to_string()],
        });
        
        // Add foreign keys
        lookup_table.add_foreign_key(ForeignKeyDefinition {
            column: format!("{}_id", relationship.from_collection),
            referenced_table: relationship.from_collection.clone(),
            referenced_column: "id".to_string(),
            constraint_name: format!("fk_{}_{}", lookup_table_name, relationship.from_collection),
        });
        
        lookup_table.add_foreign_key(ForeignKeyDefinition {
            column: format!("{}_id", relationship.to_collection),
            referenced_table: relationship.to_collection.clone(),
            referenced_column: "id".to_string(),
            constraint_name: format!("fk_{}_{}", lookup_table_name, relationship.to_collection),
        });
        
        schema.tables.push(lookup_table);
        
        // Add relationships
        schema.relationships.push(Relationship {
            from_table: lookup_table_name.clone(),
            to_table: relationship.from_collection.clone(),
            from_column: format!("{}_id", relationship.from_collection),
            to_column: "id".to_string(),
            relationship_type: RelationshipType::ManyToOne,
        });
        
        schema.relationships.push(Relationship {
            from_table: lookup_table_name,
            to_table: relationship.to_collection.clone(),
            from_column: format!("{}_id", relationship.to_collection),
            to_column: "id".to_string(),
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
            
            // Add unique index for primary key if not already present
            if let Some(ref pk) = table.primary_key {
                let pk_index_name = format!("pk_{}", table.name);
                table.add_index(IndexDefinition {
                    name: pk_index_name,
                    columns: pk.columns.clone(),
                    unique: true,
                    index_type: Some("btree".to_string()),
                });
            }
        }
        
        Ok(())
    }
}

impl Default for NormalizationEngine {
    fn default() -> Self {
        Self::new()
    }
}