use crate::types::{
    NormalizedSchema, TableDefinition, ColumnDefinition, IndexDefinition, PostgreSQLType,
    SchemaAnalysis, FieldTypeAnalysis, ConstraintType
};
use crate::error::{FireupResult, FireupError};
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
#[derive(Debug, Clone)]
pub enum PerformanceImpact {
    High,    // Significant query performance improvement
    Medium,  // Moderate query performance improvement
    Low,     // Minor query performance improvement
}

/// Storage overhead assessment for an index
#[derive(Debug, Clone)]
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
        let mut indexes = Vec::new();
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
        let mut indexes = Vec::new();
        let mut recommendations = Vec::new();

        // Partial indexes are complex and require query pattern analysis
        // For now, we'll generate recommendations for common patterns
        for table in &schema.tables {
            for column in &table.columns {
                // Recommend partial indexes for boolean flags
                if matches!(column.column_type, PostgreSQLType::Boolean) {
                    let mut partial_index = IndexDefinition {
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
        let mut table = TableDefinition::new("users".to_string());
        table.add_column(ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid));
        table.add_column(ColumnDefinition::new("email".to_string(), PostgreSQLType::Varchar(Some(255))));
        table.add_column(ColumnDefinition::new("status".to_string(), PostgreSQLType::Varchar(Some(50))));
        table.add_column(ColumnDefinition::new("created_at".to_string(), PostgreSQLType::Timestamp));
        table.add_column(ColumnDefinition::new("active".to_string(), PostgreSQLType::Boolean));
        
        // Add a foreign key
        table.add_foreign_key(ForeignKeyDefinition {
            column: "profile_id".to_string(),
            referenced_table: "profiles".to_string(),
            referenced_column: "id".to_string(),
            constraint_name: "fk_users_profile".to_string(),
        });
        
        NormalizedSchema {
            tables: vec![table],
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

    fn create_test_analysis() -> SchemaAnalysis {
        let mut analysis = SchemaAnalysis::new();
        
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "users.email".to_string(),
            type_frequencies: [("string".to_string(), 100)].iter().cloned().collect(),
            total_occurrences: 100,
            presence_percentage: 98.0,
            recommended_type: PostgreSQLType::Varchar(Some(255)),
        });
        
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "users.status".to_string(),
            type_frequencies: [("string".to_string(), 100)].iter().cloned().collect(),
            total_occurrences: 100,
            presence_percentage: 95.0,
            recommended_type: PostgreSQLType::Varchar(Some(50)),
        });
        
        analysis
    }

    #[test]
    fn test_generate_foreign_key_indexes() {
        let generator = IndexGenerator::new();
        let schema = create_test_schema();
        
        let indexes = generator.generate_foreign_key_indexes(&schema).unwrap();
        
        // Should generate index for profile_id foreign key
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].columns[0], "profile_id");
        assert_eq!(indexes[0].name, "idx_users_profile_id");
    }

    #[test]
    fn test_generate_frequent_field_indexes() {
        let generator = IndexGenerator::new();
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let (indexes, recommendations) = generator.generate_frequent_field_indexes(&schema, &analysis).unwrap();
        
        // Should generate or recommend indexes for email, status, created_at
        let total_suggestions = indexes.len() + recommendations.len();
        assert!(total_suggestions > 0);
        
        // Check that email field gets indexed (common query field with high presence)
        let email_indexed = indexes.iter().any(|idx| idx.columns.contains(&"email".to_string())) ||
                           recommendations.iter().any(|rec| rec.index.columns.contains(&"email".to_string()));
        assert!(email_indexed);
    }

    #[test]
    fn test_generate_complete_indexes() {
        let generator = IndexGenerator::new();
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let result = generator.generate_indexes(&schema, &analysis).unwrap();
        
        assert!(result.statistics.single_column_count > 0);
        assert!(result.statistics.recommendation_count > 0);
        
        // Should have foreign key indexes
        let fk_indexes: Vec<_> = result.indexes
            .iter()
            .filter(|idx| idx.columns.contains(&"profile_id".to_string()))
            .collect();
        assert!(!fk_indexes.is_empty());
    }

    #[test]
    fn test_composite_index_candidates() {
        let generator = IndexGenerator::new();
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let (_, recommendations) = generator.generate_composite_indexes(&schema, &analysis).unwrap();
        
        // Should recommend composite index for status + created_at
        let composite_rec = recommendations.iter().find(|rec| 
            rec.index.columns.len() > 1 && 
            rec.index.columns.contains(&"status".to_string()) &&
            rec.index.columns.contains(&"created_at".to_string())
        );
        
        assert!(composite_rec.is_some());
    }
}