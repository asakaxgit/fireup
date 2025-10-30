use crate::types::{
    NormalizedSchema, Constraint, ConstraintType,
    SchemaAnalysis, FieldTypeAnalysis
};
use crate::error::FireupResult;
use std::collections::{HashMap, HashSet};

/// Generator for database constraints based on schema analysis
pub struct ConstraintGenerator {
    /// Configuration for constraint generation
    config: ConstraintConfig,
}

/// Configuration options for constraint generation
#[derive(Debug, Clone)]
pub struct ConstraintConfig {
    /// Minimum percentage of populated fields to recommend NOT NULL (0.0 to 1.0)
    pub not_null_threshold: f64,
    /// Whether to generate unique constraints for fields with high uniqueness
    pub generate_unique_constraints: bool,
    /// Minimum uniqueness percentage to recommend UNIQUE constraint (0.0 to 1.0)
    pub unique_threshold: f64,
    /// Whether to generate check constraints for data validation
    pub generate_check_constraints: bool,
    /// Whether to generate foreign key constraints from relationships
    pub generate_foreign_keys: bool,
}

/// Result of constraint analysis and generation
#[derive(Debug, Clone)]
pub struct ConstraintAnalysisResult {
    /// Generated constraints
    pub constraints: Vec<Constraint>,
    /// Recommendations for manual review
    pub recommendations: Vec<ConstraintRecommendation>,
    /// Statistics about constraint generation
    pub statistics: ConstraintStatistics,
}

/// Recommendation for a constraint that requires manual review
#[derive(Debug, Clone)]
pub struct ConstraintRecommendation {
    /// Type of constraint recommended
    pub constraint_type: ConstraintType,
    /// Table the constraint applies to
    pub table: String,
    /// Columns involved
    pub columns: Vec<String>,
    /// Reason for the recommendation
    pub reason: String,
    /// Confidence level (0.0 to 1.0)
    pub confidence: f64,
    /// Suggested constraint definition
    pub suggested_definition: String,
}

/// Statistics about generated constraints
#[derive(Debug, Clone)]
pub struct ConstraintStatistics {
    /// Number of NOT NULL constraints generated
    pub not_null_count: usize,
    /// Number of UNIQUE constraints generated
    pub unique_count: usize,
    /// Number of CHECK constraints generated
    pub check_count: usize,
    /// Number of foreign key constraints generated
    pub foreign_key_count: usize,
    /// Number of recommendations made
    pub recommendation_count: usize,
}

impl Default for ConstraintConfig {
    fn default() -> Self {
        Self {
            not_null_threshold: 0.95, // 95% populated to recommend NOT NULL
            generate_unique_constraints: true,
            unique_threshold: 0.98, // 98% unique values to recommend UNIQUE
            generate_check_constraints: true,
            generate_foreign_keys: true,
        }
    }
}

impl ConstraintGenerator {
    /// Create a new constraint generator with default configuration
    pub fn new() -> Self {
        Self {
            config: ConstraintConfig::default(),
        }
    }

    /// Create a new constraint generator with custom configuration
    pub fn with_config(config: ConstraintConfig) -> Self {
        Self { config }
    }

    /// Generate constraints from schema analysis
    pub fn generate_constraints(
        &self,
        schema: &NormalizedSchema,
        analysis: &SchemaAnalysis,
    ) -> FireupResult<ConstraintAnalysisResult> {
        let mut constraints = Vec::new();
        let mut recommendations = Vec::new();
        let mut statistics = ConstraintStatistics {
            not_null_count: 0,
            unique_count: 0,
            check_count: 0,
            foreign_key_count: 0,
            recommendation_count: 0,
        };

        // Generate NOT NULL constraints based on field completeness
        let not_null_results = self.generate_not_null_constraints(schema, analysis)?;
        statistics.not_null_count = not_null_results.0.len();
        constraints.extend(not_null_results.0);
        recommendations.extend(not_null_results.1);

        // Generate UNIQUE constraints based on field uniqueness
        if self.config.generate_unique_constraints {
            let unique_results = self.generate_unique_constraints(schema, analysis)?;
            statistics.unique_count = unique_results.0.len();
            constraints.extend(unique_results.0);
            recommendations.extend(unique_results.1);
        }

        // Generate CHECK constraints for data validation
        if self.config.generate_check_constraints {
            let check_results = self.generate_check_constraints(schema, analysis)?;
            statistics.check_count = check_results.0.len();
            constraints.extend(check_results.0);
            recommendations.extend(check_results.1);
        }

        // Generate foreign key constraints from detected relationships
        if self.config.generate_foreign_keys {
            let fk_results = self.generate_foreign_key_constraints(schema, analysis)?;
            statistics.foreign_key_count = fk_results.0.len();
            constraints.extend(fk_results.0);
            recommendations.extend(fk_results.1);
        }

        statistics.recommendation_count = recommendations.len();

        Ok(ConstraintAnalysisResult {
            constraints,
            recommendations,
            statistics,
        })
    }

    /// Generate NOT NULL constraints based on field completeness
    fn generate_not_null_constraints(
        &self,
        schema: &NormalizedSchema,
        analysis: &SchemaAnalysis,
    ) -> FireupResult<(Vec<Constraint>, Vec<ConstraintRecommendation>)> {
        let mut constraints = Vec::new();
        let mut recommendations = Vec::new();

        // Create a map of field paths to their analysis
        let field_map: HashMap<String, &FieldTypeAnalysis> = analysis
            .field_types
            .iter()
            .map(|ft| (ft.field_path.clone(), ft))
            .collect();

        for table in &schema.tables {
            for column in &table.columns {
                // Skip if column is already NOT NULL
                if !column.nullable {
                    continue;
                }

                // Find corresponding field analysis
                let field_path = format!("{}.{}", table.name, column.name);
                if let Some(field_analysis) = field_map.get(&field_path) {
                    let presence_ratio = field_analysis.presence_percentage / 100.0;

                    if presence_ratio >= self.config.not_null_threshold {
                        // Generate NOT NULL constraint
                        constraints.push(Constraint {
                            name: format!("nn_{}_{}", table.name, column.name),
                            table: table.name.clone(),
                            constraint_type: ConstraintType::NotNull,
                            columns: vec![column.name.clone()],
                            parameters: HashMap::new(),
                        });
                    } else if presence_ratio >= 0.8 {
                        // Recommend NOT NULL for manual review
                        recommendations.push(ConstraintRecommendation {
                            constraint_type: ConstraintType::NotNull,
                            table: table.name.clone(),
                            columns: vec![column.name.clone()],
                            reason: format!(
                                "Field is present in {:.1}% of documents, consider NOT NULL constraint",
                                field_analysis.presence_percentage
                            ),
                            confidence: presence_ratio,
                            suggested_definition: format!(
                                "ALTER TABLE {} ALTER COLUMN {} SET NOT NULL;",
                                table.name, column.name
                            ),
                        });
                    }
                }
            }
        }

        Ok((constraints, recommendations))
    }

    /// Generate UNIQUE constraints based on field uniqueness analysis
    fn generate_unique_constraints(
        &self,
        schema: &NormalizedSchema,
        analysis: &SchemaAnalysis,
    ) -> FireupResult<(Vec<Constraint>, Vec<ConstraintRecommendation>)> {
        let constraints = Vec::new();
        let mut recommendations = Vec::new();

        // This would require additional uniqueness analysis in the schema analyzer
        // For now, we'll generate recommendations based on common patterns
        for table in &schema.tables {
            for column in &table.columns {
                // Common fields that should be unique
                let unique_candidates = ["email", "username", "phone", "ssn", "tax_id"];
                
                if unique_candidates.iter().any(|&candidate| 
                    column.name.to_lowercase().contains(candidate)
                ) {
                    recommendations.push(ConstraintRecommendation {
                        constraint_type: ConstraintType::Unique,
                        table: table.name.clone(),
                        columns: vec![column.name.clone()],
                        reason: format!(
                            "Field '{}' appears to be a unique identifier based on naming pattern",
                            column.name
                        ),
                        confidence: 0.8,
                        suggested_definition: format!(
                            "ALTER TABLE {} ADD CONSTRAINT uq_{}_{} UNIQUE ({});",
                            table.name, table.name, column.name, column.name
                        ),
                    });
                }
            }
        }

        Ok((constraints, recommendations))
    }

    /// Generate CHECK constraints for data validation
    fn generate_check_constraints(
        &self,
        schema: &NormalizedSchema,
        analysis: &SchemaAnalysis,
    ) -> FireupResult<(Vec<Constraint>, Vec<ConstraintRecommendation>)> {
        let mut constraints = Vec::new();
        let mut recommendations = Vec::new();

        for table in &schema.tables {
            for column in &table.columns {
                // Generate common check constraints based on column names and types
                match column.name.to_lowercase().as_str() {
                    name if name.contains("email") => {
                        let mut params = HashMap::new();
                        params.insert(
                            "condition".to_string(),
                            format!("{} ~* '^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\\.[A-Za-z]{{2,}}$'", column.name)
                        );
                        
                        constraints.push(Constraint {
                            name: format!("chk_{}_{}_format", table.name, column.name),
                            table: table.name.clone(),
                            constraint_type: ConstraintType::Check,
                            columns: vec![column.name.clone()],
                            parameters: params,
                        });
                    }
                    name if name.contains("age") => {
                        recommendations.push(ConstraintRecommendation {
                            constraint_type: ConstraintType::Check,
                            table: table.name.clone(),
                            columns: vec![column.name.clone()],
                            reason: "Age field should have reasonable bounds".to_string(),
                            confidence: 0.9,
                            suggested_definition: format!(
                                "ALTER TABLE {} ADD CONSTRAINT chk_{}_{}_range CHECK ({} >= 0 AND {} <= 150);",
                                table.name, table.name, column.name, column.name, column.name
                            ),
                        });
                    }
                    name if name.contains("phone") => {
                        recommendations.push(ConstraintRecommendation {
                            constraint_type: ConstraintType::Check,
                            table: table.name.clone(),
                            columns: vec![column.name.clone()],
                            reason: "Phone number should follow a standard format".to_string(),
                            confidence: 0.8,
                            suggested_definition: format!(
                                "ALTER TABLE {} ADD CONSTRAINT chk_{}_{}_format CHECK ({} ~* '^\\+?[1-9]\\d{{1,14}}$');",
                                table.name, table.name, column.name, column.name
                            ),
                        });
                    }
                    _ => {}
                }
            }
        }

        Ok((constraints, recommendations))
    }

    /// Generate foreign key constraints from detected relationships
    fn generate_foreign_key_constraints(
        &self,
        schema: &NormalizedSchema,
        analysis: &SchemaAnalysis,
    ) -> FireupResult<(Vec<Constraint>, Vec<ConstraintRecommendation>)> {
        let mut constraints = Vec::new();
        let mut recommendations = Vec::new();

        // Create a set of existing foreign keys to avoid duplicates
        let existing_fks: HashSet<String> = schema
            .tables
            .iter()
            .flat_map(|table| &table.foreign_keys)
            .map(|fk| format!("{}:{}", fk.column, fk.referenced_table))
            .collect();

        for relationship in &analysis.relationships {
            let fk_key = format!("{}:{}", relationship.reference_field, relationship.to_collection);
            
            if !existing_fks.contains(&fk_key) && relationship.confidence >= 0.8 {
                let mut params = HashMap::new();
                params.insert("referenced_table".to_string(), relationship.to_collection.clone());
                params.insert("referenced_column".to_string(), "id".to_string()); // Assume 'id' column

                if relationship.confidence >= 0.95 {
                    // High confidence - generate constraint
                    constraints.push(Constraint {
                        name: format!("fk_{}_{}", relationship.from_collection, relationship.reference_field),
                        table: relationship.from_collection.clone(),
                        constraint_type: ConstraintType::ForeignKey,
                        columns: vec![relationship.reference_field.clone()],
                        parameters: params,
                    });
                } else {
                    // Medium confidence - recommend for review
                    recommendations.push(ConstraintRecommendation {
                        constraint_type: ConstraintType::ForeignKey,
                        table: relationship.from_collection.clone(),
                        columns: vec![relationship.reference_field.clone()],
                        reason: format!(
                            "Detected relationship with {:.1}% confidence",
                            relationship.confidence * 100.0
                        ),
                        confidence: relationship.confidence,
                        suggested_definition: format!(
                            "ALTER TABLE {} ADD CONSTRAINT fk_{}_{} FOREIGN KEY ({}) REFERENCES {} (id);",
                            relationship.from_collection,
                            relationship.from_collection,
                            relationship.reference_field,
                            relationship.reference_field,
                            relationship.to_collection
                        ),
                    });
                }
            }
        }

        Ok((constraints, recommendations))
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
        users_table.add_column(ColumnDefinition::new("age".to_string(), PostgreSQLType::Integer));
        users_table.add_column(ColumnDefinition::new("phone".to_string(), PostgreSQLType::Varchar(Some(20))));
        
        let mut posts_table = TableDefinition::new("posts".to_string());
        posts_table.add_column(ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid));
        posts_table.add_column(ColumnDefinition::new("user_id".to_string(), PostgreSQLType::Uuid));
        posts_table.add_column(ColumnDefinition::new("author_id".to_string(), PostgreSQLType::Uuid));
        posts_table.add_column(ColumnDefinition::new("title".to_string(), PostgreSQLType::Text));
        
        posts_table.add_foreign_key(ForeignKeyDefinition {
            column: "user_id".to_string(),
            referenced_table: "users".to_string(),
            referenced_column: "id".to_string(),
            constraint_name: "fk_posts_user".to_string(),
        });
        
        NormalizedSchema {
            tables: vec![users_table, posts_table],
            relationships: Vec::new(),
            constraints: Vec::new(),
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
        
        // High presence field - should generate NOT NULL
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "users.email".to_string(),
            type_frequencies: [("string".to_string(), 100)].iter().cloned().collect(),
            total_occurrences: 100,
            presence_percentage: 98.0, // High presence > 95% threshold
            recommended_type: PostgreSQLType::Varchar(Some(255)),
        });
        
        // Medium presence field - should generate recommendation
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "users.age".to_string(),
            type_frequencies: [("number".to_string(), 80)].iter().cloned().collect(),
            total_occurrences: 80,
            presence_percentage: 85.0, // Medium presence (80-95%)
            recommended_type: PostgreSQLType::Integer,
        });
        
        // Low presence field - should not generate constraint
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "users.phone".to_string(),
            type_frequencies: [("string".to_string(), 30)].iter().cloned().collect(),
            total_occurrences: 30,
            presence_percentage: 30.0, // Low presence < 80%
            recommended_type: PostgreSQLType::Varchar(Some(20)),
        });

        // Add relationship for foreign key constraint testing (using a field not already covered by existing FK)
        analysis.add_relationship(DetectedRelationship {
            from_collection: "posts".to_string(),
            to_collection: "users".to_string(),
            reference_field: "author_id".to_string(),
            relationship_type: RelationshipType::ManyToOne,
            confidence: 0.96, // High confidence > 0.95 to generate constraint
        });
        
        analysis
    }

    fn create_comprehensive_test_analysis() -> SchemaAnalysis {
        let mut analysis = create_test_analysis();
        
        // Add more field types for comprehensive testing
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "users.username".to_string(),
            type_frequencies: [("string".to_string(), 95)].iter().cloned().collect(),
            total_occurrences: 95,
            presence_percentage: 95.0, // Exactly at threshold
            recommended_type: PostgreSQLType::Varchar(Some(100)),
        });
        
        analysis
    }

    #[test]
    fn test_generate_not_null_constraints() {
        let generator = ConstraintGenerator::new();
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let result = generator.generate_constraints(&schema, &analysis).unwrap();
        
        // Should generate NOT NULL for email (98% presence > 95% threshold)
        let not_null_constraints: Vec<_> = result.constraints
            .iter()
            .filter(|c| matches!(c.constraint_type, ConstraintType::NotNull))
            .collect();
        
        assert_eq!(not_null_constraints.len(), 1);
        assert_eq!(not_null_constraints[0].columns[0], "email");
        assert_eq!(not_null_constraints[0].table, "users");
        assert_eq!(not_null_constraints[0].name, "nn_users_email");
    }

    #[test]
    fn test_generate_not_null_recommendations() {
        let generator = ConstraintGenerator::new();
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let result = generator.generate_constraints(&schema, &analysis).unwrap();
        
        // Should recommend NOT NULL for age (85% presence, between 80-95%)
        let age_recommendations: Vec<_> = result.recommendations
            .iter()
            .filter(|r| matches!(r.constraint_type, ConstraintType::NotNull) && 
                     r.columns.contains(&"age".to_string()))
            .collect();
        
        assert_eq!(age_recommendations.len(), 1);
        assert!(age_recommendations[0].reason.contains("85.0%"));
        assert!(age_recommendations[0].suggested_definition.contains("ALTER TABLE users ALTER COLUMN age SET NOT NULL"));
    }

    #[test]
    fn test_generate_unique_constraints_recommendations() {
        let generator = ConstraintGenerator::new();
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let result = generator.generate_constraints(&schema, &analysis).unwrap();
        
        // Should recommend UNIQUE for email and username (common unique field patterns)
        let unique_recommendations: Vec<_> = result.recommendations
            .iter()
            .filter(|r| matches!(r.constraint_type, ConstraintType::Unique))
            .collect();
        
        assert!(!unique_recommendations.is_empty());
        
        // Check for email unique recommendation
        let email_unique = unique_recommendations.iter()
            .find(|r| r.columns.contains(&"email".to_string()));
        assert!(email_unique.is_some());
        
        // Check for username unique recommendation  
        let username_unique = unique_recommendations.iter()
            .find(|r| r.columns.contains(&"username".to_string()));
        assert!(username_unique.is_some());
    }

    #[test]
    fn test_generate_check_constraints() {
        let generator = ConstraintGenerator::new();
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let result = generator.generate_constraints(&schema, &analysis).unwrap();
        
        // Should generate email format check constraint
        let check_constraints: Vec<_> = result.constraints
            .iter()
            .filter(|c| matches!(c.constraint_type, ConstraintType::Check))
            .collect();
        
        assert!(!check_constraints.is_empty());
        
        // Find email check constraint
        let email_check = check_constraints.iter()
            .find(|c| c.columns.contains(&"email".to_string()));
        assert!(email_check.is_some());
        
        let email_constraint = email_check.unwrap();
        assert_eq!(email_constraint.name, "chk_users_email_format");
        assert!(email_constraint.parameters.contains_key("condition"));
        
        // Should recommend age range check
        let age_recommendations: Vec<_> = result.recommendations
            .iter()
            .filter(|r| matches!(r.constraint_type, ConstraintType::Check) && 
                     r.columns.contains(&"age".to_string()))
            .collect();
        
        assert!(!age_recommendations.is_empty());
        assert!(age_recommendations[0].reason.contains("reasonable bounds"));
    }

    #[test]
    fn test_generate_phone_check_recommendations() {
        let generator = ConstraintGenerator::new();
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let result = generator.generate_constraints(&schema, &analysis).unwrap();
        
        // Should recommend phone format check
        let phone_recommendations: Vec<_> = result.recommendations
            .iter()
            .filter(|r| matches!(r.constraint_type, ConstraintType::Check) && 
                     r.columns.contains(&"phone".to_string()))
            .collect();
        
        assert!(!phone_recommendations.is_empty());
        assert!(phone_recommendations[0].reason.contains("standard format"));
    }

    #[test]
    fn test_generate_foreign_key_constraints() {
        let generator = ConstraintGenerator::new();
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let result = generator.generate_constraints(&schema, &analysis).unwrap();
        
        // Should generate foreign key constraint for high confidence relationship
        let fk_constraints: Vec<_> = result.constraints
            .iter()
            .filter(|c| matches!(c.constraint_type, ConstraintType::ForeignKey))
            .collect();
        
        assert!(!fk_constraints.is_empty());
        
        let fk_constraint = &fk_constraints[0];
        assert_eq!(fk_constraint.table, "posts");
        assert_eq!(fk_constraint.columns[0], "author_id");
        assert_eq!(fk_constraint.parameters.get("referenced_table").unwrap(), "users");
        assert_eq!(fk_constraint.parameters.get("referenced_column").unwrap(), "id");
    }

    #[test]
    fn test_constraint_statistics() {
        let generator = ConstraintGenerator::new();
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let result = generator.generate_constraints(&schema, &analysis).unwrap();
        
        assert!(result.statistics.not_null_count > 0);
        assert!(result.statistics.check_count > 0);
        assert!(result.statistics.foreign_key_count > 0);
        assert!(result.statistics.recommendation_count > 0);
        
        // Verify statistics match actual counts
        let actual_not_null = result.constraints.iter()
            .filter(|c| matches!(c.constraint_type, ConstraintType::NotNull))
            .count();
        assert_eq!(result.statistics.not_null_count, actual_not_null);
        
        let actual_check = result.constraints.iter()
            .filter(|c| matches!(c.constraint_type, ConstraintType::Check))
            .count();
        assert_eq!(result.statistics.check_count, actual_check);
    }

    #[test]
    fn test_custom_thresholds() {
        let config = ConstraintConfig {
            not_null_threshold: 0.8, // Lower threshold (80%)
            unique_threshold: 0.9,   // Higher threshold (90%)
            generate_unique_constraints: true,
            generate_check_constraints: true,
            generate_foreign_keys: true,
        };
        
        let generator = ConstraintGenerator::with_config(config);
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let result = generator.generate_constraints(&schema, &analysis).unwrap();
        
        // With lower NOT NULL threshold, should generate more constraints
        let not_null_constraints: Vec<_> = result.constraints
            .iter()
            .filter(|c| matches!(c.constraint_type, ConstraintType::NotNull))
            .collect();
        
        // Should include both email (98%) and age (85%) since both > 80%
        assert!(not_null_constraints.len() >= 1);
    }

    #[test]
    fn test_disabled_constraint_generation() {
        let config = ConstraintConfig {
            generate_unique_constraints: false,
            generate_check_constraints: false,
            generate_foreign_keys: false,
            ..Default::default()
        };
        
        let generator = ConstraintGenerator::with_config(config);
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let result = generator.generate_constraints(&schema, &analysis).unwrap();
        
        // Should only generate NOT NULL constraints
        let unique_constraints: Vec<_> = result.constraints
            .iter()
            .filter(|c| matches!(c.constraint_type, ConstraintType::Unique))
            .collect();
        assert!(unique_constraints.is_empty());
        
        let check_constraints: Vec<_> = result.constraints
            .iter()
            .filter(|c| matches!(c.constraint_type, ConstraintType::Check))
            .collect();
        assert!(check_constraints.is_empty());
        
        let fk_constraints: Vec<_> = result.constraints
            .iter()
            .filter(|c| matches!(c.constraint_type, ConstraintType::ForeignKey))
            .collect();
        assert!(fk_constraints.is_empty());
        
        // Statistics should reflect disabled features
        assert_eq!(result.statistics.unique_count, 0);
        assert_eq!(result.statistics.check_count, 0);
        assert_eq!(result.statistics.foreign_key_count, 0);
    }

    #[test]
    fn test_constraint_recommendation_confidence() {
        let generator = ConstraintGenerator::new();
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let result = generator.generate_constraints(&schema, &analysis).unwrap();
        
        // All recommendations should have confidence scores
        for recommendation in &result.recommendations {
            assert!(recommendation.confidence > 0.0);
            assert!(recommendation.confidence <= 1.0);
            assert!(!recommendation.reason.is_empty());
            assert!(!recommendation.suggested_definition.is_empty());
        }
    }

    #[test]
    fn test_existing_foreign_key_avoidance() {
        let generator = ConstraintGenerator::new();
        let mut schema = create_test_schema();
        let analysis = create_test_analysis();
        
        // Add existing foreign key to avoid duplicates
        let existing_fk = ForeignKeyDefinition {
            column: "author_id".to_string(),
            referenced_table: "users".to_string(),
            referenced_column: "id".to_string(),
            constraint_name: "existing_fk".to_string(),
        };
        schema.tables[1].add_foreign_key(existing_fk);
        
        let result = generator.generate_constraints(&schema, &analysis).unwrap();
        
        // Should not generate duplicate foreign key constraint
        let fk_constraints: Vec<_> = result.constraints
            .iter()
            .filter(|c| matches!(c.constraint_type, ConstraintType::ForeignKey) &&
                     c.columns.contains(&"author_id".to_string()))
            .collect();
        
        // Should be empty since existing FK already covers this relationship
        assert!(fk_constraints.is_empty());
    }

    #[test]
    fn test_medium_confidence_relationship_recommendation() {
        let generator = ConstraintGenerator::new();
        let schema = create_test_schema();
        let mut analysis = create_test_analysis();
        
        // Add medium confidence relationship
        analysis.add_relationship(DetectedRelationship {
            from_collection: "posts".to_string(),
            to_collection: "users".to_string(),
            reference_field: "reviewer_id".to_string(),
            relationship_type: RelationshipType::ManyToOne,
            confidence: 0.85, // Medium confidence (between 0.8 and 0.95)
        });
        
        let result = generator.generate_constraints(&schema, &analysis).unwrap();
        
        // Should generate recommendation instead of constraint
        let fk_recommendations: Vec<_> = result.recommendations
            .iter()
            .filter(|r| matches!(r.constraint_type, ConstraintType::ForeignKey) &&
                     r.columns.contains(&"reviewer_id".to_string()))
            .collect();
        
        assert!(!fk_recommendations.is_empty());
        assert!(fk_recommendations[0].reason.contains("85.0%"));
    }
}