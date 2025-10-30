use crate::types::{
    NormalizedSchema, TableDefinition, ColumnDefinition, IndexDefinition, PostgreSQLType,
    SchemaAnalysis, FieldTypeAnalysis, ConstraintType
};
use crate::error::FireupResult;
use std::collections::{HashMap, HashSet};

/// Generator for database indexes based on schema analysis and usage patterns
pub struct IndexGenerator {
    /// Configuration for index generation
    config: IndexConfig,
}

/// Configuration options for index generation
#[derive(Debug, Clone)]
pub struct IndexConfig {
    /// Whether to generate indexes for foreign key columns
    pub index_foreign_keys: bool,
    /// Whether to generate indexes for frequently queried fields
    pub index_frequent_fields: bool,
    /// Whether to generate unique indexes for unique constraints
    pub index_unique_constraints: bool,
    /// Whether to generate composite indexes for related fields
    pub generate_composite_indexes: bool,
    /// Maximum number of columns in a composite index
    pub max_composite_columns: usize,
    /// Whether to generate partial indexes for filtered queries
    pub generate_partial_indexes: bool,
}

/// Result of index analysis and generation
#[derive(Debug, Clone)]
pub struct IndexAnalysisResult {
    /// Generated index definitions
    pub indexes: Vec<IndexDefinition>,
    /// Recommendations for manual review
    pub recommendations: Vec<IndexRecommendation>,
    /// Statistics about index generation
    pub statistics: IndexStatistics,
}

/// Recommendation for an index that requires manual review
#[derive(Debug, Clone)]
pub struct IndexRecommendation {
    /// Recommended index definition
    pub index: IndexDefinition,
    /// Reason for the recommendation
    pub reason: String,
    /// Confidence level (0.0 to 1.0)
    pub confidence: f64,
    /// Estimated performance impact
    pub performance_impact: PerformanceImpact,
    /// Estimated storage overhead
    pub storage_overhead: StorageOverhead,
}

/// Performance impact assessment for an index
#[derive(Debug, Clone, PartialEq)]
pub enum PerformanceImpact {
    High,    // Significant query performance improvement
    Medium,  // Moderate query performance improvement
    Low,     // Minor query performance improvement
}

/// Storage overhead assessment for an index
#[derive(Debug, Clone, PartialEq)]
pub enum StorageOverhead {
    High,    // Significant storage space required
    Medium,  // Moderate storage space required
    Low,     // Minimal storage space required
}

/// Statistics about generated indexes
#[derive(Debug, Clone)]
pub struct IndexStatistics {
    /// Number of single-column indexes generated
    pub single_column_count: usize,
    /// Number of composite indexes generated
    pub composite_count: usize,
    /// Number of unique indexes generated
    pub unique_count: usize,
    /// Number of partial indexes generated
    pub partial_count: usize,
    /// Number of recommendations made
    pub recommendation_count: usize,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            index_foreign_keys: true,
            index_frequent_fields: true,
            index_unique_constraints: true,
            generate_composite_indexes: true,
            max_composite_columns: 3,
            generate_partial_indexes: false, // Conservative default
        }
    }
}

impl IndexGenerator {
    /// Create a new index generator with default configuration
    pub fn new() -> Self {
        Self {
            config: IndexConfig::default(),
        }
    }

    /// Create a new index generator with custom configuration
    pub fn with_config(config: IndexConfig) -> Self {
        Self { config }
    }

    /// Generate indexes from schema analysis
    pub fn generate_indexes(
        &self,
        schema: &NormalizedSchema,
        analysis: &SchemaAnalysis,
    ) -> FireupResult<IndexAnalysisResult> {
        let mut indexes = Vec::new();
        let mut recommendations = Vec::new();
        let mut statistics = IndexStatistics {
            single_column_count: 0,
            composite_count: 0,
            unique_count: 0,
            partial_count: 0,
            recommendation_count: 0,
        };

        // Generate indexes for foreign key columns
        if self.config.index_foreign_keys {
            let fk_indexes = self.generate_foreign_key_indexes(schema)?;
            statistics.single_column_count += fk_indexes.len();
            indexes.extend(fk_indexes);
        }

        // Generate indexes for unique constraints
        if self.config.index_unique_constraints {
            let unique_indexes = self.generate_unique_constraint_indexes(schema)?;
            statistics.unique_count += unique_indexes.len();
            indexes.extend(unique_indexes);
        }

        // Generate indexes for frequently accessed fields
        if self.config.index_frequent_fields {
            let (frequent_indexes, frequent_recommendations) = 
                self.generate_frequent_field_indexes(schema, analysis)?;
            statistics.single_column_count += frequent_indexes.len();
            indexes.extend(frequent_indexes);
            recommendations.extend(frequent_recommendations);
        }

        // Generate composite indexes for related fields
        if self.config.generate_composite_indexes {
            let (composite_indexes, composite_recommendations) = 
                self.generate_composite_indexes(schema, analysis)?;
            statistics.composite_count += composite_indexes.len();
            indexes.extend(composite_indexes);
            recommendations.extend(composite_recommendations);
        }

        // Generate partial indexes for filtered queries
        if self.config.generate_partial_indexes {
            let (partial_indexes, partial_recommendations) = 
                self.generate_partial_indexes(schema, analysis)?;
            statistics.partial_count += partial_indexes.len();
            indexes.extend(partial_indexes);
            recommendations.extend(partial_recommendations);
        }

        statistics.recommendation_count = recommendations.len();

        Ok(IndexAnalysisResult {
            indexes,
            recommendations,
            statistics,
        })
    }

    /// Generate indexes for foreign key columns
    fn generate_foreign_key_indexes(&self, schema: &NormalizedSchema) -> FireupResult<Vec<IndexDefinition>> {
        let mut indexes = Vec::new();
        let mut existing_indexes = HashSet::new();

        for table in &schema.tables {
            // Collect existing index columns to avoid duplicates
            for index in &table.indexes {
                if index.columns.len() == 1 {
                    existing_indexes.insert(format!("{}:{}", table.name, index.columns[0]));
                }
            }

            // Create indexes for foreign key columns
            for fk in &table.foreign_keys {
                let index_key = format!("{}:{}", table.name, fk.column);
                
                if !existing_indexes.contains(&index_key) {
                    indexes.push(IndexDefinition {
                        name: format!("idx_{}_{}", table.name, fk.column),
                        columns: vec![fk.column.clone()],
                        unique: false,
                        index_type: Some("BTREE".to_string()),
                    });
                    existing_indexes.insert(index_key);
                }
            }
        }

        Ok(indexes)
    }

    /// Generate indexes for unique constraints
    fn generate_unique_constraint_indexes(&self, schema: &NormalizedSchema) -> FireupResult<Vec<IndexDefinition>> {
        let mut indexes = Vec::new();
        let mut existing_indexes = HashSet::new();

        for table in &schema.tables {
            // Collect existing unique indexes
            for index in &table.indexes {
                if index.unique {
                    existing_indexes.insert(format!("{}:{}", table.name, index.columns.join(",")));
                }
            }
        }

        // Generate indexes for unique constraints
        for constraint in &schema.constraints {
            if matches!(constraint.constraint_type, ConstraintType::Unique) {
                let index_key = format!("{}:{}", constraint.table, constraint.columns.join(","));
                
                if !existing_indexes.contains(&index_key) {
                    indexes.push(IndexDefinition {
                        name: format!("idx_unique_{}_{}", constraint.table, constraint.columns.join("_")),
                        columns: constraint.columns.clone(),
                        unique: true,
                        index_type: Some("BTREE".to_string()),
                    });
                }
            }
        }

        Ok(indexes)
    }

    /// Generate indexes for frequently accessed fields
    fn generate_frequent_field_indexes(
        &self,
        schema: &NormalizedSchema,
        analysis: &SchemaAnalysis,
    ) -> FireupResult<(Vec<IndexDefinition>, Vec<IndexRecommendation>)> {
        let mut indexes = Vec::new();
        let mut recommendations = Vec::new();

        // Create a map of field paths to their analysis
        let field_map: HashMap<String, &FieldTypeAnalysis> = analysis
            .field_types
            .iter()
            .map(|ft| (ft.field_path.clone(), ft))
            .collect();

        for table in &schema.tables {
            let mut existing_indexes = HashSet::new();
            
            // Collect existing single-column indexes
            for index in &table.indexes {
                if index.columns.len() == 1 {
                    existing_indexes.insert(index.columns[0].clone());
                }
            }

            for column in &table.columns {
                let field_path = format!("{}.{}", table.name, column.name);
                
                if let Some(field_analysis) = field_map.get(&field_path) {
                    // Skip if index already exists
                    if existing_indexes.contains(&column.name) {
                        continue;
                    }

                    // Generate indexes based on field characteristics
                    let should_index = self.should_index_field(column, field_analysis);
                    
                    if should_index.0 {
                        if should_index.1 >= 0.8 {
                            // High confidence - generate index
                            indexes.push(IndexDefinition {
                                name: format!("idx_{}_{}", table.name, column.name),
                                columns: vec![column.name.clone()],
                                unique: false,
                                index_type: Some("BTREE".to_string()),
                            });
                        } else {
                            // Medium confidence - recommend for review
                            recommendations.push(IndexRecommendation {
                                index: IndexDefinition {
                                    name: format!("idx_{}_{}", table.name, column.name),
                                    columns: vec![column.name.clone()],
                                    unique: false,
                                    index_type: Some("BTREE".to_string()),
                                },
                                reason: self.get_index_reason(column, field_analysis),
                                confidence: should_index.1,
                                performance_impact: self.assess_performance_impact(column, field_analysis),
                                storage_overhead: self.assess_storage_overhead(column, field_analysis),
                            });
                        }
                    }
                }
            }
        }

        Ok((indexes, recommendations))
    }

    /// Generate composite indexes for related fields
    fn generate_composite_indexes(
        &self,
        schema: &NormalizedSchema,
        analysis: &SchemaAnalysis,
    ) -> FireupResult<(Vec<IndexDefinition>, Vec<IndexRecommendation>)> {
        let indexes = Vec::new();
        let mut recommendations = Vec::new();

        for table in &schema.tables {
            // Generate composite indexes for common query patterns
            let composite_candidates = self.identify_composite_candidates(table, analysis)?;
            
            for candidate in composite_candidates {
                if candidate.columns.len() <= self.config.max_composite_columns {
                    let recommendation = IndexRecommendation {
                        index: candidate,
                        reason: "Composite index for related fields that are likely queried together".to_string(),
                        confidence: 0.7, // Medium confidence for composite indexes
                        performance_impact: PerformanceImpact::Medium,
                        storage_overhead: StorageOverhead::Medium,
                    };
                    recommendations.push(recommendation);
                }
            }
        }

        Ok((indexes, recommendations))
    }

    /// Generate partial indexes for filtered queries
    fn generate_partial_indexes(
        &self,
        schema: &NormalizedSchema,
        analysis: &SchemaAnalysis,
    ) -> FireupResult<(Vec<IndexDefinition>, Vec<IndexRecommendation>)> {
        let indexes = Vec::new();
        let mut recommendations = Vec::new();

        // Partial indexes are complex and require query pattern analysis
        // For now, we'll generate recommendations for common patterns
        for table in &schema.tables {
            for column in &table.columns {
                // Recommend partial indexes for boolean flags
                if matches!(column.column_type, PostgreSQLType::Boolean) {
                    let partial_index = IndexDefinition {
                        name: format!("idx_{}_{}_true", table.name, column.name),
                        columns: vec![column.name.clone()],
                        unique: false,
                        index_type: Some("BTREE".to_string()),
                    };

                    recommendations.push(IndexRecommendation {
                        index: partial_index,
                        reason: format!("Partial index for boolean field '{}' where value is TRUE", column.name),
                        confidence: 0.6,
                        performance_impact: PerformanceImpact::Medium,
                        storage_overhead: StorageOverhead::Low,
                    });
                }
            }
        }

        Ok((indexes, recommendations))
    }

    /// Determine if a field should be indexed
    fn should_index_field(&self, column: &ColumnDefinition, field_analysis: &FieldTypeAnalysis) -> (bool, f64) {
        let mut score = 0.0;
        let mut reasons = Vec::new();

        // High presence suggests frequent access
        if field_analysis.presence_percentage > 90.0 {
            score += 0.3;
            reasons.push("high presence");
        }

        // Common query fields based on naming patterns
        let query_field_patterns = [
            "id", "email", "username", "name", "status", "type", "category",
            "created_at", "updated_at", "date", "timestamp", "active", "enabled"
        ];

        for pattern in &query_field_patterns {
            if column.name.to_lowercase().contains(pattern) {
                score += 0.4;
                reasons.push("common query field pattern");
                break;
            }
        }

        // String fields are often used in WHERE clauses
        if matches!(column.column_type, PostgreSQLType::Varchar(_) | PostgreSQLType::Text) {
            score += 0.2;
            reasons.push("string field");
        }

        // Timestamp fields are often used for sorting and filtering
        if matches!(column.column_type, PostgreSQLType::Timestamp) {
            score += 0.3;
            reasons.push("timestamp field");
        }

        // Boolean fields are often used for filtering
        if matches!(column.column_type, PostgreSQLType::Boolean) {
            score += 0.2;
            reasons.push("boolean field");
        }

        (score > 0.5, (score as f64).min(1.0))
    }

    /// Get reason for indexing a field
    fn get_index_reason(&self, column: &ColumnDefinition, field_analysis: &FieldTypeAnalysis) -> String {
        let mut reasons = Vec::new();

        if field_analysis.presence_percentage > 90.0 {
            reasons.push("high field presence");
        }

        let query_patterns = ["id", "email", "username", "status", "created_at", "updated_at"];
        for pattern in &query_patterns {
            if column.name.to_lowercase().contains(pattern) {
                reasons.push("common query field");
                break;
            }
        }

        if matches!(column.column_type, PostgreSQLType::Timestamp) {
            reasons.push("timestamp field for sorting/filtering");
        }

        if reasons.is_empty() {
            "field characteristics suggest frequent querying".to_string()
        } else {
            format!("Recommended due to: {}", reasons.join(", "))
        }
    }

    /// Assess performance impact of an index
    fn assess_performance_impact(&self, column: &ColumnDefinition, field_analysis: &FieldTypeAnalysis) -> PerformanceImpact {
        // High impact for frequently accessed fields
        if field_analysis.presence_percentage > 95.0 {
            return PerformanceImpact::High;
        }

        // High impact for common query fields
        let high_impact_patterns = ["id", "email", "username", "created_at", "updated_at"];
        for pattern in &high_impact_patterns {
            if column.name.to_lowercase().contains(pattern) {
                return PerformanceImpact::High;
            }
        }

        // Medium impact for other string and timestamp fields
        if matches!(column.column_type, PostgreSQLType::Varchar(_) | PostgreSQLType::Text | PostgreSQLType::Timestamp) {
            PerformanceImpact::Medium
        } else {
            PerformanceImpact::Low
        }
    }

    /// Assess storage overhead of an index
    fn assess_storage_overhead(&self, column: &ColumnDefinition, _field_analysis: &FieldTypeAnalysis) -> StorageOverhead {
        match &column.column_type {
            PostgreSQLType::Text => StorageOverhead::High,
            PostgreSQLType::Varchar(Some(len)) if *len > 100 => StorageOverhead::Medium,
            PostgreSQLType::Varchar(_) => StorageOverhead::Low,
            PostgreSQLType::Jsonb => StorageOverhead::High,
            PostgreSQLType::Array(_) => StorageOverhead::Medium,
            _ => StorageOverhead::Low,
        }
    }

    /// Identify candidates for composite indexes
    fn identify_composite_candidates(&self, table: &TableDefinition, _analysis: &SchemaAnalysis) -> FireupResult<Vec<IndexDefinition>> {
        let mut candidates = Vec::new();

        // Common composite index patterns
        let composite_patterns = [
            // Status + timestamp combinations
            (vec!["status", "created_at"], "status and creation time"),
            (vec!["status", "updated_at"], "status and update time"),
            (vec!["active", "created_at"], "active status and creation time"),
            
            // User identification patterns
            (vec!["user_id", "created_at"], "user and creation time"),
            (vec!["user_id", "status"], "user and status"),
            
            // Category + date patterns
            (vec!["category", "created_at"], "category and creation time"),
            (vec!["type", "created_at"], "type and creation time"),
        ];

        for (pattern_columns, description) in &composite_patterns {
            let matching_columns: Vec<String> = table.columns
                .iter()
                .filter_map(|col| {
                    for pattern in pattern_columns {
                        if col.name.to_lowercase().contains(pattern) {
                            return Some(col.name.clone());
                        }
                    }
                    None
                })
                .collect();

            if matching_columns.len() >= 2 {
                candidates.push(IndexDefinition {
                    name: format!("idx_{}_{}", table.name, matching_columns.join("_")),
                    columns: matching_columns,
                    unique: false,
                    index_type: Some("BTREE".to_string()),
                });
            }
        }

        Ok(candidates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn create_test_schema() -> NormalizedSchema {
        let mut users_table = TableDefinition::new("users".to_string());
        users_table.add_column(ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid));
        users_table.add_column(ColumnDefinition::new("email".to_string(), PostgreSQLType::Varchar(Some(255))));
        users_table.add_column(ColumnDefinition::new("username".to_string(), PostgreSQLType::Varchar(Some(100))));
        users_table.add_column(ColumnDefinition::new("status".to_string(), PostgreSQLType::Varchar(Some(50))));
        users_table.add_column(ColumnDefinition::new("created_at".to_string(), PostgreSQLType::Timestamp));
        users_table.add_column(ColumnDefinition::new("updated_at".to_string(), PostgreSQLType::Timestamp));
        users_table.add_column(ColumnDefinition::new("active".to_string(), PostgreSQLType::Boolean));
        users_table.add_column(ColumnDefinition::new("category".to_string(), PostgreSQLType::Varchar(Some(100))));
        
        // Add existing index to test duplicate avoidance
        users_table.add_index(IndexDefinition {
            name: "idx_users_existing".to_string(),
            columns: vec!["email".to_string()],
            unique: true,
            index_type: Some("BTREE".to_string()),
        });
        
        // Add foreign keys
        users_table.add_foreign_key(ForeignKeyDefinition {
            column: "profile_id".to_string(),
            referenced_table: "profiles".to_string(),
            referenced_column: "id".to_string(),
            constraint_name: "fk_users_profile".to_string(),
        });
        
        users_table.add_foreign_key(ForeignKeyDefinition {
            column: "department_id".to_string(),
            referenced_table: "departments".to_string(),
            referenced_column: "id".to_string(),
            constraint_name: "fk_users_department".to_string(),
        });

        let mut posts_table = TableDefinition::new("posts".to_string());
        posts_table.add_column(ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid));
        posts_table.add_column(ColumnDefinition::new("user_id".to_string(), PostgreSQLType::Uuid));
        posts_table.add_column(ColumnDefinition::new("title".to_string(), PostgreSQLType::Text));
        posts_table.add_column(ColumnDefinition::new("type".to_string(), PostgreSQLType::Varchar(Some(50))));
        posts_table.add_column(ColumnDefinition::new("created_at".to_string(), PostgreSQLType::Timestamp));
        posts_table.add_column(ColumnDefinition::new("enabled".to_string(), PostgreSQLType::Boolean));
        
        posts_table.add_foreign_key(ForeignKeyDefinition {
            column: "user_id".to_string(),
            referenced_table: "users".to_string(),
            referenced_column: "id".to_string(),
            constraint_name: "fk_posts_user".to_string(),
        });
        
        // Add unique constraints for testing
        let mut constraints = Vec::new();
        constraints.push(Constraint {
            name: "uq_users_username".to_string(),
            table: "users".to_string(),
            constraint_type: ConstraintType::Unique,
            columns: vec!["username".to_string()],
            parameters: HashMap::new(),
        });
        
        NormalizedSchema {
            tables: vec![users_table, posts_table],
            relationships: Vec::new(),
            constraints,
            warnings: Vec::new(),
            metadata: SchemaMetadata {
                generated_at: Utc::now(),
                source_analysis_id: Uuid::new_v4(),
                version: "1.0.0".to_string(),
                table_count: 2,
                relationship_count: 1,
            },
        }
    }

    fn create_test_analysis() -> SchemaAnalysis {
        let mut analysis = SchemaAnalysis::new();
        
        // High presence fields
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "users.email".to_string(),
            type_frequencies: [("string".to_string(), 100)].iter().cloned().collect(),
            total_occurrences: 100,
            presence_percentage: 98.0, // High presence
            recommended_type: PostgreSQLType::Varchar(Some(255)),
        });
        
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "users.username".to_string(),
            type_frequencies: [("string".to_string(), 95)].iter().cloned().collect(),
            total_occurrences: 95,
            presence_percentage: 95.0, // High presence
            recommended_type: PostgreSQLType::Varchar(Some(100)),
        });
        
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "users.status".to_string(),
            type_frequencies: [("string".to_string(), 100)].iter().cloned().collect(),
            total_occurrences: 100,
            presence_percentage: 95.0, // High presence
            recommended_type: PostgreSQLType::Varchar(Some(50)),
        });
        
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "users.created_at".to_string(),
            type_frequencies: [("timestamp".to_string(), 100)].iter().cloned().collect(),
            total_occurrences: 100,
            presence_percentage: 100.0, // Perfect presence
            recommended_type: PostgreSQLType::Timestamp,
        });
        
        // Medium presence field
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "users.category".to_string(),
            type_frequencies: [("string".to_string(), 80)].iter().cloned().collect(),
            total_occurrences: 80,
            presence_percentage: 80.0, // Medium presence
            recommended_type: PostgreSQLType::Varchar(Some(100)),
        });
        
        // Posts table fields
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "posts.title".to_string(),
            type_frequencies: [("string".to_string(), 100)].iter().cloned().collect(),
            total_occurrences: 100,
            presence_percentage: 100.0,
            recommended_type: PostgreSQLType::Text,
        });
        
        analysis
    }

    #[test]
    fn test_generate_foreign_key_indexes() {
        let generator = IndexGenerator::new();
        let schema = create_test_schema();
        
        let indexes = generator.generate_foreign_key_indexes(&schema).unwrap();
        
        // Should generate indexes for foreign keys (profile_id, department_id, user_id)
        // but not for email since it already has an index
        assert_eq!(indexes.len(), 3);
        
        let index_columns: Vec<&String> = indexes.iter()
            .flat_map(|idx| &idx.columns)
            .collect();
        
        assert!(index_columns.contains(&&"profile_id".to_string()));
        assert!(index_columns.contains(&&"department_id".to_string()));
        assert!(index_columns.contains(&&"user_id".to_string()));
        
        // Check index naming
        let profile_index = indexes.iter()
            .find(|idx| idx.columns.contains(&"profile_id".to_string()))
            .unwrap();
        assert_eq!(profile_index.name, "idx_users_profile_id");
        assert!(!profile_index.unique);
        assert_eq!(profile_index.index_type, Some("BTREE".to_string()));
    }

    #[test]
    fn test_generate_unique_constraint_indexes() {
        let generator = IndexGenerator::new();
        let schema = create_test_schema();
        
        let indexes = generator.generate_unique_constraint_indexes(&schema).unwrap();
        
        // Should generate unique index for username constraint
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].columns[0], "username");
        assert!(indexes[0].unique);
        assert_eq!(indexes[0].name, "idx_unique_users_username");
    }

    #[test]
    fn test_generate_frequent_field_indexes() {
        let generator = IndexGenerator::new();
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let (indexes, recommendations) = generator.generate_frequent_field_indexes(&schema, &analysis).unwrap();
        
        // Should generate or recommend indexes for common query fields
        let total_suggestions = indexes.len() + recommendations.len();
        assert!(total_suggestions > 0);
        
        // Check that common query fields get indexed or recommended
        let all_suggested_columns: Vec<String> = indexes.iter()
            .chain(recommendations.iter().map(|r| &r.index))
            .flat_map(|idx| &idx.columns)
            .cloned()
            .collect();
        
        // Should suggest indexes for timestamp fields (created_at, updated_at)
        assert!(all_suggested_columns.iter().any(|col| col.contains("created_at")));
        
        // Should suggest indexes for status fields
        assert!(all_suggested_columns.iter().any(|col| col == "status"));
        
        // Should NOT suggest index for email since it already exists
        let email_suggestions = indexes.iter()
            .chain(recommendations.iter().map(|r| &r.index))
            .filter(|idx| idx.columns.contains(&"email".to_string()))
            .count();
        assert_eq!(email_suggestions, 0);
    }

    #[test]
    fn test_should_index_field_logic() {
        let generator = IndexGenerator::new();
        
        // Test high presence field with common pattern
        let email_column = ColumnDefinition::new("email".to_string(), PostgreSQLType::Varchar(Some(255)));
        let email_analysis = FieldTypeAnalysis {
            field_path: "users.email".to_string(),
            type_frequencies: [("string".to_string(), 100)].iter().cloned().collect(),
            total_occurrences: 100,
            presence_percentage: 98.0,
            recommended_type: PostgreSQLType::Varchar(Some(255)),
        };
        
        let (should_index, confidence) = generator.should_index_field(&email_column, &email_analysis);
        assert!(should_index);
        assert!(confidence > 0.8);
        
        // Test timestamp field
        let timestamp_column = ColumnDefinition::new("created_at".to_string(), PostgreSQLType::Timestamp);
        let timestamp_analysis = FieldTypeAnalysis {
            field_path: "users.created_at".to_string(),
            type_frequencies: [("timestamp".to_string(), 100)].iter().cloned().collect(),
            total_occurrences: 100,
            presence_percentage: 100.0,
            recommended_type: PostgreSQLType::Timestamp,
        };
        
        let (should_index, confidence) = generator.should_index_field(&timestamp_column, &timestamp_analysis);
        assert!(should_index);
        assert!(confidence > 0.7);
        
        // Test low presence field without common pattern
        let rare_column = ColumnDefinition::new("rare_field".to_string(), PostgreSQLType::Text);
        let rare_analysis = FieldTypeAnalysis {
            field_path: "users.rare_field".to_string(),
            type_frequencies: [("string".to_string(), 10)].iter().cloned().collect(),
            total_occurrences: 10,
            presence_percentage: 10.0,
            recommended_type: PostgreSQLType::Text,
        };
        
        let (should_index, confidence) = generator.should_index_field(&rare_column, &rare_analysis);
        assert!(!should_index);
        assert!(confidence <= 0.5);
    }

    #[test]
    fn test_generate_composite_indexes() {
        let generator = IndexGenerator::new();
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let (indexes, recommendations) = generator.generate_composite_indexes(&schema, &analysis).unwrap();
        
        // Should recommend composite indexes for related fields
        assert!(!recommendations.is_empty());
        
        // Should find status + created_at combination
        let status_created_composite = recommendations.iter().find(|rec| 
            rec.index.columns.len() > 1 && 
            rec.index.columns.contains(&"status".to_string()) &&
            rec.index.columns.contains(&"created_at".to_string())
        );
        assert!(status_created_composite.is_some());
        
        // Should find type + created_at combination in posts table
        let type_created_composite = recommendations.iter().find(|rec| 
            rec.index.columns.len() > 1 && 
            rec.index.columns.contains(&"type".to_string()) &&
            rec.index.columns.contains(&"created_at".to_string())
        );
        assert!(type_created_composite.is_some());
    }

    #[test]
    fn test_generate_partial_indexes() {
        let generator = IndexGenerator::with_config(IndexConfig {
            generate_partial_indexes: true,
            ..Default::default()
        });
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let (indexes, recommendations) = generator.generate_partial_indexes(&schema, &analysis).unwrap();
        
        // Should recommend partial indexes for boolean fields
        let boolean_partials: Vec<_> = recommendations.iter()
            .filter(|rec| rec.index.columns.iter().any(|col| col == "active" || col == "enabled"))
            .collect();
        
        assert!(!boolean_partials.is_empty());
        
        // Check partial index properties
        let active_partial = boolean_partials.iter()
            .find(|rec| rec.index.columns.contains(&"active".to_string()));
        
        if let Some(partial) = active_partial {
            assert!(partial.index.name.contains("_true"));
            assert_eq!(partial.performance_impact, PerformanceImpact::Medium);
            assert_eq!(partial.storage_overhead, StorageOverhead::Low);
        }
    }

    #[test]
    fn test_generate_complete_indexes() {
        let generator = IndexGenerator::new();
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let result = generator.generate_indexes(&schema, &analysis).unwrap();
        
        // Verify statistics
        assert!(result.statistics.single_column_count > 0);
        assert!(result.statistics.unique_count > 0);
        assert!(result.statistics.recommendation_count > 0);
        
        // Should have foreign key indexes
        let fk_indexes: Vec<_> = result.indexes
            .iter()
            .filter(|idx| idx.columns.iter().any(|col| col.ends_with("_id")))
            .collect();
        assert!(!fk_indexes.is_empty());
        
        // Should have unique constraint indexes
        let unique_indexes: Vec<_> = result.indexes
            .iter()
            .filter(|idx| idx.unique)
            .collect();
        assert!(!unique_indexes.is_empty());
        
        // Verify statistics match actual counts
        let actual_single_column = result.indexes.iter()
            .filter(|idx| idx.columns.len() == 1)
            .count();
        
        let actual_unique = result.indexes.iter()
            .filter(|idx| idx.unique)
            .count();
        
        assert_eq!(result.statistics.unique_count, actual_unique);
    }

    #[test]
    fn test_performance_impact_assessment() {
        let generator = IndexGenerator::new();
        
        // High impact field (email with high presence)
        let email_column = ColumnDefinition::new("email".to_string(), PostgreSQLType::Varchar(Some(255)));
        let email_analysis = FieldTypeAnalysis {
            field_path: "users.email".to_string(),
            type_frequencies: [("string".to_string(), 100)].iter().cloned().collect(),
            total_occurrences: 100,
            presence_percentage: 98.0,
            recommended_type: PostgreSQLType::Varchar(Some(255)),
        };
        
        let impact = generator.assess_performance_impact(&email_column, &email_analysis);
        assert!(matches!(impact, PerformanceImpact::High));
        
        // Medium impact field (timestamp)
        let timestamp_column = ColumnDefinition::new("some_date".to_string(), PostgreSQLType::Timestamp);
        let timestamp_analysis = FieldTypeAnalysis {
            field_path: "users.some_date".to_string(),
            type_frequencies: [("timestamp".to_string(), 80)].iter().cloned().collect(),
            total_occurrences: 80,
            presence_percentage: 80.0,
            recommended_type: PostgreSQLType::Timestamp,
        };
        
        let impact = generator.assess_performance_impact(&timestamp_column, &timestamp_analysis);
        assert!(matches!(impact, PerformanceImpact::Medium));
        
        // Low impact field
        let low_column = ColumnDefinition::new("misc_field".to_string(), PostgreSQLType::Integer);
        let low_analysis = FieldTypeAnalysis {
            field_path: "users.misc_field".to_string(),
            type_frequencies: [("integer".to_string(), 50)].iter().cloned().collect(),
            total_occurrences: 50,
            presence_percentage: 50.0,
            recommended_type: PostgreSQLType::Integer,
        };
        
        let impact = generator.assess_performance_impact(&low_column, &low_analysis);
        assert!(matches!(impact, PerformanceImpact::Low));
    }

    #[test]
    fn test_storage_overhead_assessment() {
        let generator = IndexGenerator::new();
        let dummy_analysis = FieldTypeAnalysis {
            field_path: "test.field".to_string(),
            type_frequencies: [("string".to_string(), 100)].iter().cloned().collect(),
            total_occurrences: 100,
            presence_percentage: 100.0,
            recommended_type: PostgreSQLType::Text,
        };
        
        // High overhead (TEXT)
        let text_column = ColumnDefinition::new("content".to_string(), PostgreSQLType::Text);
        let overhead = generator.assess_storage_overhead(&text_column, &dummy_analysis);
        assert!(matches!(overhead, StorageOverhead::High));
        
        // High overhead (JSONB)
        let jsonb_column = ColumnDefinition::new("data".to_string(), PostgreSQLType::Jsonb);
        let overhead = generator.assess_storage_overhead(&jsonb_column, &dummy_analysis);
        assert!(matches!(overhead, StorageOverhead::High));
        
        // Medium overhead (long VARCHAR)
        let long_varchar_column = ColumnDefinition::new("description".to_string(), PostgreSQLType::Varchar(Some(500)));
        let overhead = generator.assess_storage_overhead(&long_varchar_column, &dummy_analysis);
        assert!(matches!(overhead, StorageOverhead::Medium));
        
        // Low overhead (short VARCHAR)
        let short_varchar_column = ColumnDefinition::new("code".to_string(), PostgreSQLType::Varchar(Some(10)));
        let overhead = generator.assess_storage_overhead(&short_varchar_column, &dummy_analysis);
        assert!(matches!(overhead, StorageOverhead::Low));
        
        // Low overhead (UUID)
        let uuid_column = ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid);
        let overhead = generator.assess_storage_overhead(&uuid_column, &dummy_analysis);
        assert!(matches!(overhead, StorageOverhead::Low));
    }

    #[test]
    fn test_custom_index_config() {
        let config = IndexConfig {
            index_foreign_keys: false,
            index_frequent_fields: true,
            index_unique_constraints: false,
            generate_composite_indexes: false,
            max_composite_columns: 2,
            generate_partial_indexes: false,
        };
        
        let generator = IndexGenerator::with_config(config);
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let result = generator.generate_indexes(&schema, &analysis).unwrap();
        
        // Should not generate foreign key indexes
        let fk_indexes: Vec<_> = result.indexes
            .iter()
            .filter(|idx| idx.columns.iter().any(|col| col.ends_with("_id")))
            .collect();
        assert!(fk_indexes.is_empty());
        
        // Should not generate unique constraint indexes
        assert_eq!(result.statistics.unique_count, 0);
        
        // Should not generate composite indexes
        assert_eq!(result.statistics.composite_count, 0);
        
        // Should not generate partial indexes
        assert_eq!(result.statistics.partial_count, 0);
        
        // Should still generate frequent field indexes
        assert!(result.statistics.single_column_count > 0 || result.statistics.recommendation_count > 0);
    }

    #[test]
    fn test_max_composite_columns_limit() {
        let config = IndexConfig {
            generate_composite_indexes: true,
            max_composite_columns: 2,
            ..Default::default()
        };
        
        let generator = IndexGenerator::with_config(config);
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let (_, recommendations) = generator.generate_composite_indexes(&schema, &analysis).unwrap();
        
        // All composite index recommendations should respect the column limit
        for recommendation in &recommendations {
            assert!(recommendation.index.columns.len() <= 2);
        }
    }

    #[test]
    fn test_index_recommendation_properties() {
        let generator = IndexGenerator::new();
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let result = generator.generate_indexes(&schema, &analysis).unwrap();
        
        // All recommendations should have proper properties
        for recommendation in &result.recommendations {
            assert!(!recommendation.reason.is_empty());
            assert!(recommendation.confidence > 0.0);
            assert!(recommendation.confidence <= 1.0);
            assert!(!recommendation.index.name.is_empty());
            assert!(!recommendation.index.columns.is_empty());
            
            // Performance impact and storage overhead should be assessed
            match recommendation.performance_impact {
                PerformanceImpact::High | PerformanceImpact::Medium | PerformanceImpact::Low => {}
            }
            
            match recommendation.storage_overhead {
                StorageOverhead::High | StorageOverhead::Medium | StorageOverhead::Low => {}
            }
        }
    }
}