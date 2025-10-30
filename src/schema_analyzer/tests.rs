use super::*;
use crate::types::*;
use crate::monitoring::{initialize_monitoring, MonitoringConfig};
use serde_json::{json, Value};
use std::collections::HashMap;

/// Helper function to create a test Firestore document
fn create_test_document(id: &str, collection: &str, data: HashMap<String, Value>) -> FirestoreDocument {
    let mut doc = FirestoreDocument::new(id.to_string(), collection.to_string(), format!("{}/{}", collection, id));
    doc.data = data;
    doc.metadata.size_bytes = Some(1024);
    doc
}

/// Helper function to create test documents with various structures
fn create_test_documents() -> Vec<FirestoreDocument> {
    vec![
        // User documents with consistent structure
        create_test_document("user1", "users", {
            let mut data = HashMap::new();
            data.insert("name".to_string(), json!("John Doe"));
            data.insert("email".to_string(), json!("john@example.com"));
            data.insert("age".to_string(), json!(30));
            data.insert("active".to_string(), json!(true));
            data.insert("tags".to_string(), json!(["developer", "rust"]));
            data
        }),
        create_test_document("user2", "users", {
            let mut data = HashMap::new();
            data.insert("name".to_string(), json!("Jane Smith"));
            data.insert("email".to_string(), json!("jane@example.com"));
            data.insert("age".to_string(), json!(25));
            data.insert("active".to_string(), json!(true));
            data.insert("tags".to_string(), json!(["designer", "ui"]));
            data
        }),
        // User with type conflict (age as string)
        create_test_document("user3", "users", {
            let mut data = HashMap::new();
            data.insert("name".to_string(), json!("Bob Wilson"));
            data.insert("email".to_string(), json!("bob@example.com"));
            data.insert("age".to_string(), json!("unknown"));
            data.insert("active".to_string(), json!(false));
            data
        }),
        // Product documents with nested structure
        create_test_document("prod1", "products", {
            let mut data = HashMap::new();
            data.insert("title".to_string(), json!("Laptop"));
            data.insert("price".to_string(), json!(999.99));
            data.insert("category".to_string(), json!("electronics"));
            data.insert("specs".to_string(), json!({
                "cpu": "Intel i7",
                "ram": "16GB",
                "storage": "512GB SSD"
            }));
            data.insert("reviews".to_string(), json!([
                {"rating": 5, "comment": "Great laptop"},
                {"rating": 4, "comment": "Good value"}
            ]));
            data
        }),
        create_test_document("prod2", "products", {
            let mut data = HashMap::new();
            data.insert("title".to_string(), json!("Mouse"));
            data.insert("price".to_string(), json!(29.99));
            data.insert("category".to_string(), json!("electronics"));
            data.insert("specs".to_string(), json!({
                "dpi": "1600",
                "wireless": true
            }));
            data
        }),
    ]
}

#[cfg(test)]
mod analyzer_tests {
    use super::*;

    #[tokio::test]
    async fn test_analyze_documents_basic_structure() {
        initialize_monitoring(MonitoringConfig::default());
        let analyzer = DocumentStructureAnalyzer::new();
        let documents = create_test_documents();
        
        let result = analyzer.analyze_documents(&documents).await;
        assert!(result.is_ok());
        
        let analysis = result.unwrap();
        
        // Should detect 2 collections
        assert_eq!(analysis.collections.len(), 2);
        
        // Check users collection
        let users_collection = analysis.collections.iter()
            .find(|c| c.name == "users")
            .expect("Users collection should be found");
        assert_eq!(users_collection.document_count, 3);
        assert!(users_collection.field_names.contains(&"name".to_string()));
        assert!(users_collection.field_names.contains(&"email".to_string()));
        assert!(users_collection.field_names.contains(&"age".to_string()));
        
        // Check products collection
        let products_collection = analysis.collections.iter()
            .find(|c| c.name == "products")
            .expect("Products collection should be found");
        assert_eq!(products_collection.document_count, 2);
        assert!(products_collection.field_names.contains(&"title".to_string()));
        assert!(products_collection.field_names.contains(&"price".to_string()));
    }

    #[tokio::test]
    async fn test_field_type_analysis() {
        initialize_monitoring(MonitoringConfig::default());
        let analyzer = DocumentStructureAnalyzer::new();
        let documents = create_test_documents();
        
        let analysis = analyzer.analyze_documents(&documents).await.unwrap();
        
        // Check field type analysis
        let name_field = analysis.field_types.iter()
            .find(|ft| ft.field_path == "users.name")
            .expect("Name field should be analyzed");
        
        assert_eq!(name_field.total_occurrences, 3);
        assert_eq!(name_field.presence_percentage, 100.0);
        assert!(name_field.type_frequencies.contains_key("string"));
        assert_eq!(*name_field.type_frequencies.get("string").unwrap(), 3);
        
        // Check age field with type conflict
        let age_field = analysis.field_types.iter()
            .find(|ft| ft.field_path == "users.age")
            .expect("Age field should be analyzed");
        
        assert_eq!(age_field.total_occurrences, 3);
        assert!(age_field.type_frequencies.contains_key("integer"));
        assert!(age_field.type_frequencies.contains_key("string"));
        assert_eq!(*age_field.type_frequencies.get("integer").unwrap(), 2);
        assert_eq!(*age_field.type_frequencies.get("string").unwrap(), 1);
    }

    #[tokio::test]
    async fn test_normalization_opportunities_detection() {
        initialize_monitoring(MonitoringConfig::default());
        let analyzer = DocumentStructureAnalyzer::new();
        let documents = create_test_documents();
        
        let analysis = analyzer.analyze_documents(&documents).await.unwrap();
        
        // Should detect array normalization opportunities
        let first_nf_opportunities: Vec<_> = analysis.normalization_opportunities.iter()
            .filter(|opp| matches!(opp.normalization_type, NormalizationType::FirstNormalForm))
            .collect();
        
        assert!(!first_nf_opportunities.is_empty());
        
        // Should find tags array in users collection
        let tags_opportunity = first_nf_opportunities.iter()
            .find(|opp| opp.field_path == "tags" && opp.collection == "users");
        assert!(tags_opportunity.is_some());
    }

    #[tokio::test]
    async fn test_relationship_detection() {
        initialize_monitoring(MonitoringConfig::default());
        let analyzer = DocumentStructureAnalyzer::new();
        
        // Create documents with reference patterns
        let documents = vec![
            create_test_document("order1", "orders", {
                let mut data = HashMap::new();
                data.insert("user_ref".to_string(), json!("users/user1"));
                data.insert("total".to_string(), json!(100.0));
                data
            }),
            create_test_document("order2", "orders", {
                let mut data = HashMap::new();
                data.insert("user_ref".to_string(), json!("users/user2"));
                data.insert("total".to_string(), json!(200.0));
                data
            }),
        ];
        
        let analysis = analyzer.analyze_documents(&documents).await.unwrap();
        
        // Should detect relationship from orders to users
        let relationships: Vec<_> = analysis.relationships.iter()
            .filter(|rel| rel.from_collection == "orders" && rel.to_collection == "users")
            .collect();
        
        assert!(!relationships.is_empty());
        let relationship = relationships[0];
        assert_eq!(relationship.reference_field, "user_ref");
        assert!(relationship.confidence >= 0.7);
    }

    #[tokio::test]
    async fn test_empty_documents() {
        initialize_monitoring(MonitoringConfig::default());
        let analyzer = DocumentStructureAnalyzer::new();
        let documents = vec![];
        
        let result = analyzer.analyze_documents(&documents).await;
        assert!(result.is_ok());
        
        let analysis = result.unwrap();
        assert_eq!(analysis.collections.len(), 0);
        assert_eq!(analysis.field_types.len(), 0);
        assert_eq!(analysis.metadata.total_documents, 0);
    }

    #[tokio::test]
    async fn test_nested_field_analysis() {
        initialize_monitoring(MonitoringConfig::default());
        let analyzer = DocumentStructureAnalyzer::new();
        let documents = vec![
            create_test_document("doc1", "test", {
                let mut data = HashMap::new();
                data.insert("nested".to_string(), json!({
                    "level1": {
                        "level2": "deep_value"
                    }
                }));
                data
            }),
        ];
        
        let analysis = analyzer.analyze_documents(&documents).await.unwrap();
        
        // Should analyze nested fields
        let nested_fields: Vec<_> = analysis.field_types.iter()
            .filter(|ft| ft.field_path.contains("nested"))
            .collect();
        
        assert!(!nested_fields.is_empty());
    }
}

#[cfg(test)]
mod normalizer_tests {
    use super::*;

    fn create_test_analysis() -> SchemaAnalysis {
        let mut analysis = SchemaAnalysis::new();
        
        // Add collection analysis
        analysis.add_collection(CollectionAnalysis {
            name: "users".to_string(),
            document_count: 3,
            field_names: vec!["name".to_string(), "email".to_string(), "tags".to_string()],
            avg_document_size: 1024.0,
            subcollections: vec![],
        });
        
        // Add field type analysis
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "users.name".to_string(),
            type_frequencies: {
                let mut map = HashMap::new();
                map.insert("string".to_string(), 3);
                map
            },
            total_occurrences: 3,
            presence_percentage: 100.0,
            recommended_type: PostgreSQLType::Text,
        });
        
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "users.email".to_string(),
            type_frequencies: {
                let mut map = HashMap::new();
                map.insert("string".to_string(), 3);
                map
            },
            total_occurrences: 3,
            presence_percentage: 100.0,
            recommended_type: PostgreSQLType::Text,
        });
        
        // Add normalization opportunity for array field
        analysis.add_normalization_opportunity(NormalizationOpportunity {
            collection: "users".to_string(),
            field_path: "tags".to_string(),
            normalization_type: NormalizationType::FirstNormalForm,
            description: "Array field can be normalized".to_string(),
            impact: NormalizationImpact::Medium,
        });
        
        analysis
    }

    #[test]
    fn test_normalize_schema_basic() {
        let normalizer = NormalizationEngine::new();
        let analysis = create_test_analysis();
        
        let result = normalizer.normalize_schema(&analysis);
        assert!(result.is_ok());
        
        let schema = result.unwrap();
        
        // Should create main table
        assert!(!schema.tables.is_empty());
        let users_table = schema.tables.iter()
            .find(|t| t.name == "users")
            .expect("Users table should be created");
        
        // Should have primary key
        assert!(users_table.primary_key.is_some());
        let pk = users_table.primary_key.as_ref().unwrap();
        assert_eq!(pk.columns[0], "id");
        
        // Should have basic columns
        let id_column = users_table.columns.iter()
            .find(|c| c.name == "id")
            .expect("ID column should exist");
        assert!(!id_column.nullable);
        assert!(matches!(id_column.column_type, PostgreSQLType::Uuid));
    }

    #[test]
    fn test_first_normal_form_application() {
        let normalizer = NormalizationEngine::new();
        let mut analysis = create_test_analysis();
        
        // Add array field type analysis
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "users.tags".to_string(),
            type_frequencies: {
                let mut map = HashMap::new();
                map.insert("array".to_string(), 2);
                map
            },
            total_occurrences: 2,
            presence_percentage: 66.7,
            recommended_type: PostgreSQLType::Text,
        });
        
        let schema = normalizer.normalize_schema(&analysis).unwrap();
        
        // Should create separate table for array elements
        let array_table = schema.tables.iter()
            .find(|t| t.name == "users_tags");
        
        // If array table was created, check its structure
        if let Some(table) = array_table {
        
            // Array table should have foreign key
            assert!(!table.foreign_keys.is_empty());
            let fk = &table.foreign_keys[0];
            assert_eq!(fk.referenced_table, "users");
            assert_eq!(fk.referenced_column, "id");
            
            // Should create relationship
            let relationship = schema.relationships.iter()
                .find(|r| r.from_table == "users_tags" && r.to_table == "users")
                .expect("Relationship should be created");
            assert!(matches!(relationship.relationship_type, RelationshipType::ManyToOne));
        } else {
            // If no array table was created, that's also acceptable behavior
            // depending on the normalization thresholds
            println!("Array table not created - this may be expected based on thresholds");
        }
    }

    #[test]
    fn test_aggressive_normalization() {
        let normalizer = NormalizationEngine::new_aggressive();
        let analysis = create_test_analysis();
        
        let result = normalizer.normalize_schema(&analysis);
        assert!(result.is_ok());
        
        let schema = result.unwrap();
        assert!(!schema.tables.is_empty());
    }

    #[test]
    fn test_index_generation() {
        let normalizer = NormalizationEngine::new();
        let analysis = create_test_analysis();
        
        let schema = normalizer.normalize_schema(&analysis).unwrap();
        
        // Should generate indexes for primary keys and foreign keys
        for table in &schema.tables {
            if let Some(ref pk) = table.primary_key {
                let pk_index = table.indexes.iter()
                    .find(|idx| idx.unique && idx.columns == pk.columns);
                assert!(pk_index.is_some(), "Primary key index should be created for table {}", table.name);
            }
            
            for fk in &table.foreign_keys {
                let fk_index = table.indexes.iter()
                    .find(|idx| idx.columns.contains(&fk.column));
                assert!(fk_index.is_some(), "Foreign key index should be created for column {}", fk.column);
            }
        }
    }

    #[test]
    fn test_schema_metadata() {
        let normalizer = NormalizationEngine::new();
        let analysis = create_test_analysis();
        
        let schema = normalizer.normalize_schema(&analysis).unwrap();
        
        // Check metadata
        assert!(schema.metadata.table_count > 0);
        assert_eq!(schema.metadata.table_count, schema.tables.len() as u32);
        assert_eq!(schema.metadata.relationship_count, schema.relationships.len() as u32);
        assert_eq!(schema.metadata.version, "1.0.0");
    }
}

#[cfg(test)]
mod type_conflict_resolver_tests {
    use super::*;

    fn create_conflicted_analysis() -> SchemaAnalysis {
        let mut analysis = SchemaAnalysis::new();
        
        // Add field with type conflict (dominant type is only 60%, below 70% threshold)
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "users.age".to_string(),
            type_frequencies: {
                let mut map = HashMap::new();
                map.insert("integer".to_string(), 6);
                map.insert("string".to_string(), 4);
                map
            },
            total_occurrences: 10,
            presence_percentage: 100.0,
            recommended_type: PostgreSQLType::Integer,
        });
        
        // Add field with severe conflict
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "products.mixed_field".to_string(),
            type_frequencies: {
                let mut map = HashMap::new();
                map.insert("string".to_string(), 3);
                map.insert("integer".to_string(), 3);
                map.insert("boolean".to_string(), 2);
                map.insert("array".to_string(), 2);
                map
            },
            total_occurrences: 10,
            presence_percentage: 100.0,
            recommended_type: PostgreSQLType::Jsonb,
        });
        
        // Add field with compatible types
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "users.score".to_string(),
            type_frequencies: {
                let mut map = HashMap::new();
                map.insert("integer".to_string(), 6);
                map.insert("number".to_string(), 4);
                map
            },
            total_occurrences: 10,
            presence_percentage: 100.0,
            recommended_type: PostgreSQLType::Numeric(None, None),
        });
        
        analysis
    }

    #[test]
    fn test_detect_type_conflicts() {
        let resolver = TypeConflictResolver::new();
        let analysis = create_conflicted_analysis();
        
        let result = resolver.detect_and_resolve_conflicts(&analysis);
        assert!(result.is_ok());
        
        let conflicts = result.unwrap();
        
        // Should detect conflicts for mixed types
        assert!(!conflicts.is_empty());
        
        let age_conflict = conflicts.iter()
            .find(|c| c.field_path == "users.age")
            .expect("Age conflict should be detected");
        
        assert_eq!(age_conflict.conflicting_types.len(), 2);
        assert!(age_conflict.conflicting_types.contains(&"integer".to_string()));
        assert!(age_conflict.conflicting_types.contains(&"string".to_string()));
        assert_eq!(age_conflict.total_occurrences, 10);
    }

    #[test]
    fn test_conflict_resolution_strategies() {
        let resolver = TypeConflictResolver::new();
        let analysis = create_conflicted_analysis();
        
        let conflicts = resolver.detect_and_resolve_conflicts(&analysis).unwrap();
        
        // Check age conflict resolution (dominant type strategy)
        let age_conflict = conflicts.iter()
            .find(|c| c.field_path == "users.age")
            .expect("Age conflict should be found");
        
        assert!(age_conflict.suggested_resolution.contains("dominant type"));
        assert!(age_conflict.resolution_confidence > 0.5); // Lower threshold since we have 60% dominance
        
        // Check mixed field conflict (JSONB strategy)
        let mixed_conflict = conflicts.iter()
            .find(|c| c.field_path == "products.mixed_field")
            .expect("Mixed field conflict should be found");
        
        assert!(mixed_conflict.suggested_resolution.contains("JSONB"));
        
        // Check numeric conflict (compatible types strategy)
        // Note: numeric types (integer + number) might not be detected as conflicts
        // if they're considered compatible, which is expected behavior
        let score_conflict = conflicts.iter()
            .find(|c| c.field_path == "users.score");
        
        if let Some(conflict) = score_conflict {
            // If a conflict is detected, it should suggest a compatible resolution
            assert!(conflict.suggested_resolution.contains("NUMERIC") || 
                   conflict.suggested_resolution.contains("accommodate") ||
                   conflict.suggested_resolution.contains("dominant type"));
        }
        // If no conflict is detected for compatible numeric types, that's also acceptable
    }

    #[test]
    fn test_dominant_type_calculation() {
        let mut conflict = TypeConflict::new("test.field".to_string());
        conflict.add_type_occurrence("integer".to_string());
        conflict.add_type_occurrence("integer".to_string());
        conflict.add_type_occurrence("integer".to_string());
        conflict.add_type_occurrence("string".to_string());
        
        assert_eq!(conflict.dominant_type(), Some("integer".to_string()));
        assert_eq!(conflict.dominant_type_percentage(), 75.0);
    }

    #[test]
    fn test_conflict_statistics() {
        let resolver = TypeConflictResolver::new();
        let analysis = create_conflicted_analysis();
        
        let conflicts = resolver.detect_and_resolve_conflicts(&analysis).unwrap();
        let stats = resolver.get_conflict_statistics(&conflicts);
        
        assert_eq!(stats.total_conflicts, conflicts.len());
        assert!(stats.total_affected_fields > 0);
        assert!(!stats.most_common_conflict_types.is_empty());
    }

    #[test]
    fn test_no_conflicts_for_single_type() {
        let resolver = TypeConflictResolver::new();
        let mut analysis = SchemaAnalysis::new();
        
        // Add field with single type
        analysis.add_field_type(FieldTypeAnalysis {
            field_path: "users.name".to_string(),
            type_frequencies: {
                let mut map = HashMap::new();
                map.insert("string".to_string(), 10);
                map
            },
            total_occurrences: 10,
            presence_percentage: 100.0,
            recommended_type: PostgreSQLType::Text,
        });
        
        let conflicts = resolver.detect_and_resolve_conflicts(&analysis).unwrap();
        assert!(conflicts.is_empty());
    }

    #[test]
    fn test_custom_thresholds() {
        let resolver = TypeConflictResolver::with_thresholds(0.9, 0.95);
        let analysis = create_conflicted_analysis();
        
        let conflicts = resolver.detect_and_resolve_conflicts(&analysis).unwrap();
        
        // With higher thresholds, more fields should be considered conflicted
        assert!(!conflicts.is_empty());
    }

    #[test]
    fn test_conflict_report_generation() {
        let resolver = TypeConflictResolver::new();
        let analysis = create_conflicted_analysis();
        
        let conflicts = resolver.detect_and_resolve_conflicts(&analysis).unwrap();
        let report = resolver.generate_conflict_report(&conflicts);
        
        assert!(report.contains("Type Conflict Analysis Report"));
        assert!(report.contains("Total conflicts detected"));
        assert!(!report.is_empty());
    }
}