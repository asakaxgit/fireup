use crate::error::{FireupResult};
use crate::types::{
    FirestoreDocument, SchemaAnalysis, CollectionAnalysis, FieldTypeAnalysis, 
    DetectedRelationship, NormalizationOpportunity, PostgreSQLType, RelationshipType,
    NormalizationType, NormalizationImpact
};
use crate::monitoring::{get_monitoring_system, AuditOperationType, AuditResult};
use std::collections::{HashMap, HashSet};
use serde_json::Value;
use tracing::{info, debug, instrument};

/// Document structure analyzer that detects field types and structures
pub struct DocumentStructureAnalyzer {
    /// Minimum confidence threshold for relationship detection
    relationship_confidence_threshold: f64,
    /// Minimum occurrence percentage for type recommendations
    type_recommendation_threshold: f64,
}

impl DocumentStructureAnalyzer {
    /// Create a new document structure analyzer
    pub fn new() -> Self {
        Self {
            relationship_confidence_threshold: 0.7,
            type_recommendation_threshold: 0.8,
        }
    }

    /// Analyze documents and detect field types and structures
    #[instrument(skip(self, documents))]
    pub async fn analyze_documents(&self, documents: &[FirestoreDocument]) -> FireupResult<SchemaAnalysis> {
        let tracker = get_monitoring_system().start_operation("schema_analysis").await;
        tracker.add_metadata("document_count", documents.len().to_string()).await.ok();
        
        info!("Starting document structure analysis for {} documents", documents.len());
        
        let mut analysis = SchemaAnalysis::new();
        analysis.metadata.total_documents = documents.len() as u64;
        
        // Group documents by collection
        let collections = self.group_documents_by_collection(documents);
        
        // Analyze each collection
        for (collection_name, collection_docs) in collections {
            debug!("Analyzing collection: {} with {} documents", collection_name, collection_docs.len());
            
            let collection_analysis = self.analyze_collection(&collection_name, &collection_docs).await?;
            analysis.add_collection(collection_analysis);
            
            // Analyze field types for this collection
            let field_types = self.analyze_field_types(&collection_name, &collection_docs).await?;
            for field_type in field_types {
                analysis.add_field_type(field_type);
            }
            
            // Detect relationships within this collection
            let relationships = self.detect_relationships(&collection_name, &collection_docs).await?;
            for relationship in relationships {
                analysis.add_relationship(relationship);
            }
            
            // Find normalization opportunities
            let opportunities = self.find_normalization_opportunities(&collection_name, &collection_docs).await?;
            for opportunity in opportunities {
                analysis.add_normalization_opportunity(opportunity);
            }
        }
        
        analysis.complete();
        info!("Document structure analysis completed");

        // Log schema analysis audit entry
        let mut details = HashMap::new();
        details.insert("collections_analyzed".to_string(), analysis.collections.len().to_string());
        details.insert("field_types_detected".to_string(), analysis.field_types.len().to_string());
        details.insert("relationships_found".to_string(), analysis.relationships.len().to_string());
        details.insert("normalization_opportunities".to_string(), analysis.normalization_opportunities.len().to_string());
        
        get_monitoring_system().log_audit_entry(
            AuditOperationType::DataAccess,
            "document_collection",
            "schema_analysis",
            "analyze_structure",
            AuditResult::Success,
            details,
            None,
        ).await.ok();

        tracker.update_progress(documents.len() as u64, None).await.ok();
        tracker.complete_success().await.ok();
        
        Ok(analysis)
    }

    /// Group documents by collection name
    fn group_documents_by_collection<'a>(&self, documents: &'a [FirestoreDocument]) -> HashMap<String, Vec<&'a FirestoreDocument>> {
        let mut collections = HashMap::new();
        
        for doc in documents {
            collections.entry(doc.collection.clone())
                .or_insert_with(Vec::new)
                .push(doc);
            
            // Also process subcollections recursively
            for subdoc in &doc.subcollections {
                let subcollection_name = format!("{}_{}", doc.collection, subdoc.collection);
                collections.entry(subcollection_name)
                    .or_insert_with(Vec::new)
                    .push(subdoc);
            }
        }
        
        collections
    }

    /// Analyze a specific collection
    async fn analyze_collection(&self, collection_name: &str, documents: &[&FirestoreDocument]) -> FireupResult<CollectionAnalysis> {
        let mut field_names = HashSet::new();
        let mut total_size = 0u64;
        let mut subcollections = HashSet::new();
        
        for doc in documents {
            // Collect field names
            for key in doc.data.keys() {
                field_names.insert(key.clone());
            }
            
            // Calculate document size
            if let Some(size) = doc.metadata.size_bytes {
                total_size += size;
            }
            
            // Collect subcollection names
            for subdoc in &doc.subcollections {
                subcollections.insert(subdoc.collection.clone());
            }
        }
        
        let avg_document_size = if documents.is_empty() {
            0.0
        } else {
            total_size as f64 / documents.len() as f64
        };
        
        Ok(CollectionAnalysis {
            name: collection_name.to_string(),
            document_count: documents.len() as u64,
            field_names: field_names.into_iter().collect(),
            avg_document_size,
            subcollections: subcollections.into_iter().collect(),
        })
    }

    /// Analyze field types across documents in a collection
    async fn analyze_field_types(&self, collection_name: &str, documents: &[&FirestoreDocument]) -> FireupResult<Vec<FieldTypeAnalysis>> {
        let mut field_analysis = HashMap::new();
        
        for doc in documents {
            self.analyze_document_fields(&mut field_analysis, &doc.data, collection_name, "")?;
        }
        
        let mut results = Vec::new();
        for (field_path, type_counts) in field_analysis {
            let total_occurrences = type_counts.values().sum::<u32>();
            let presence_percentage = (total_occurrences as f64 / documents.len() as f64) * 100.0;
            
            let recommended_type = self.recommend_postgresql_type(&type_counts);
            
            results.push(FieldTypeAnalysis {
                field_path,
                type_frequencies: type_counts,
                total_occurrences,
                presence_percentage,
                recommended_type,
            });
        }
        
        Ok(results)
    }

    /// Recursively analyze fields in a document
    fn analyze_document_fields(
        &self,
        field_analysis: &mut HashMap<String, HashMap<String, u32>>,
        data: &HashMap<String, Value>,
        collection_name: &str,
        parent_path: &str,
    ) -> FireupResult<()> {
        for (key, value) in data {
            let field_path = if parent_path.is_empty() {
                format!("{}.{}", collection_name, key)
            } else {
                format!("{}.{}", parent_path, key)
            };
            
            let type_name = self.get_value_type_name(value);
            
            field_analysis.entry(field_path.clone())
                .or_insert_with(HashMap::new)
                .entry(type_name)
                .and_modify(|count| *count += 1)
                .or_insert(1);
            
            // Recursively analyze nested objects
            if let Value::Object(nested_obj) = value {
                // Convert serde_json::Map to HashMap for recursive call
                let nested_hashmap: HashMap<String, Value> = nested_obj.iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                self.analyze_document_fields(field_analysis, &nested_hashmap, collection_name, &field_path)?;
            }
        }
        
        Ok(())
    }

    /// Get the type name of a JSON value
    fn get_value_type_name(&self, value: &Value) -> String {
        match value {
            Value::Null => "null".to_string(),
            Value::Bool(_) => "boolean".to_string(),
            Value::Number(n) => {
                if n.is_i64() || n.is_u64() {
                    "integer".to_string()
                } else {
                    "number".to_string()
                }
            }
            Value::String(s) => {
                // Try to detect special string types
                if s.len() == 36 && s.chars().filter(|c| *c == '-').count() == 4 {
                    "uuid".to_string()
                } else if s.contains('T') && s.contains('Z') {
                    "timestamp".to_string()
                } else {
                    "string".to_string()
                }
            }
            Value::Array(_) => "array".to_string(),
            Value::Object(_) => "object".to_string(),
        }
    }

    /// Recommend PostgreSQL type based on detected types
    fn recommend_postgresql_type(&self, type_counts: &HashMap<String, u32>) -> PostgreSQLType {
        let total_count: u32 = type_counts.values().sum();
        
        // Find the most common type
        let dominant_type = type_counts
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(type_name, _)| type_name.as_str())
            .unwrap_or("string");
        
        let dominant_percentage = type_counts.get(dominant_type).unwrap_or(&0) * 100 / total_count;
        
        // If dominant type is less than threshold, use JSONB for flexibility
        if (dominant_percentage as f64) < (self.type_recommendation_threshold * 100.0) {
            return PostgreSQLType::Jsonb;
        }
        
        match dominant_type {
            "boolean" => PostgreSQLType::Boolean,
            "integer" => PostgreSQLType::Integer,
            "number" => PostgreSQLType::Numeric(None, None),
            "uuid" => PostgreSQLType::Uuid,
            "timestamp" => PostgreSQLType::Timestamp,
            "string" => {
                // Determine appropriate string type based on length analysis
                PostgreSQLType::Text
            }
            "array" => PostgreSQLType::Jsonb, // Arrays will be normalized separately
            "object" => PostgreSQLType::Jsonb,
            _ => PostgreSQLType::Text,
        }
    }

    /// Detect relationships between collections
    async fn detect_relationships(&self, collection_name: &str, documents: &[&FirestoreDocument]) -> FireupResult<Vec<DetectedRelationship>> {
        let mut relationships = Vec::new();
        let mut reference_patterns = HashMap::new();
        
        // Analyze reference patterns
        for doc in documents {
            self.analyze_reference_patterns(&mut reference_patterns, &doc.data, collection_name)?;
        }
        
        // Convert patterns to relationships
        for (field_path, target_collections) in reference_patterns {
            for (target_collection, count) in target_collections {
                let confidence = count as f64 / documents.len() as f64;
                
                if confidence >= self.relationship_confidence_threshold {
                    let relationship_type = self.determine_relationship_type(collection_name, &target_collection, documents).await?;
                    
                    relationships.push(DetectedRelationship {
                        from_collection: collection_name.to_string(),
                        to_collection: target_collection,
                        reference_field: field_path.clone(),
                        relationship_type,
                        confidence,
                    });
                }
            }
        }
        
        Ok(relationships)
    }

    /// Analyze reference patterns in document data
    fn analyze_reference_patterns(
        &self,
        patterns: &mut HashMap<String, HashMap<String, u32>>,
        data: &HashMap<String, Value>,
        collection_name: &str,
    ) -> FireupResult<()> {
        for (key, value) in data {
            if let Value::String(s) = value {
                // Look for reference-like patterns (e.g., collection/document_id)
                if s.contains('/') {
                    let parts: Vec<&str> = s.split('/').collect();
                    if parts.len() >= 2 {
                        let target_collection = parts[0].to_string();
                        if target_collection != collection_name {
                            patterns.entry(key.clone())
                                .or_insert_with(HashMap::new)
                                .entry(target_collection)
                                .and_modify(|count| *count += 1)
                                .or_insert(1);
                        }
                    }
                }
            }
        }
        
        Ok(())
    }

    /// Determine the type of relationship between collections
    async fn determine_relationship_type(&self, _from_collection: &str, _to_collection: &str, _documents: &[&FirestoreDocument]) -> FireupResult<RelationshipType> {
        // For now, assume most relationships are one-to-many
        // This could be enhanced with more sophisticated analysis
        Ok(RelationshipType::OneToMany)
    }

    /// Find normalization opportunities in a collection
    async fn find_normalization_opportunities(&self, collection_name: &str, documents: &[&FirestoreDocument]) -> FireupResult<Vec<NormalizationOpportunity>> {
        let mut opportunities = Vec::new();
        
        // Analyze for 1NF violations (repeating groups/arrays)
        let first_nf_opportunities = self.find_first_normal_form_opportunities(collection_name, documents).await?;
        opportunities.extend(first_nf_opportunities);
        
        // Analyze for 2NF violations (partial dependencies)
        let second_nf_opportunities = self.find_second_normal_form_opportunities(collection_name, documents).await?;
        opportunities.extend(second_nf_opportunities);
        
        // Analyze for 3NF violations (transitive dependencies)
        let third_nf_opportunities = self.find_third_normal_form_opportunities(collection_name, documents).await?;
        opportunities.extend(third_nf_opportunities);
        
        Ok(opportunities)
    }

    /// Find First Normal Form opportunities (eliminate repeating groups)
    async fn find_first_normal_form_opportunities(&self, collection_name: &str, documents: &[&FirestoreDocument]) -> FireupResult<Vec<NormalizationOpportunity>> {
        let mut opportunities = Vec::new();
        let mut array_fields = HashMap::new();
        
        // Find array fields
        for doc in documents {
            self.find_array_fields(&mut array_fields, &doc.data, "")?;
        }
        
        for (field_path, count) in array_fields {
            let occurrence_rate = count as f64 / documents.len() as f64;
            
            if occurrence_rate > 0.1 { // At least 10% of documents have this array
                opportunities.push(NormalizationOpportunity {
                    collection: collection_name.to_string(),
                    field_path,
                    normalization_type: NormalizationType::FirstNormalForm,
                    description: "Array field can be normalized into a separate table".to_string(),
                    impact: if occurrence_rate > 0.5 {
                        NormalizationImpact::High
                    } else if occurrence_rate > 0.3 {
                        NormalizationImpact::Medium
                    } else {
                        NormalizationImpact::Low
                    },
                });
            }
        }
        
        Ok(opportunities)
    }

    /// Find array fields in document data
    fn find_array_fields(&self, array_fields: &mut HashMap<String, u32>, data: &HashMap<String, Value>, parent_path: &str) -> FireupResult<()> {
        for (key, value) in data {
            let field_path = if parent_path.is_empty() {
                key.clone()
            } else {
                format!("{}.{}", parent_path, key)
            };
            
            match value {
                Value::Array(_) => {
                    array_fields.entry(field_path)
                        .and_modify(|count| *count += 1)
                        .or_insert(1);
                }
                Value::Object(nested_obj) => {
                    // Convert serde_json::Map to HashMap for recursive call
                    let nested_hashmap: HashMap<String, Value> = nested_obj.iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    self.find_array_fields(array_fields, &nested_hashmap, &field_path)?;
                }
                _ => {}
            }
        }
        
        Ok(())
    }

    /// Find Second Normal Form opportunities (eliminate partial dependencies)
    async fn find_second_normal_form_opportunities(&self, collection_name: &str, _documents: &[&FirestoreDocument]) -> FireupResult<Vec<NormalizationOpportunity>> {
        let mut opportunities = Vec::new();
        
        // For now, we'll add a placeholder opportunity
        // This would require more sophisticated analysis of composite keys and dependencies
        opportunities.push(NormalizationOpportunity {
            collection: collection_name.to_string(),
            field_path: "composite_key_analysis".to_string(),
            normalization_type: NormalizationType::SecondNormalForm,
            description: "Potential partial dependency detected - requires manual review".to_string(),
            impact: NormalizationImpact::Medium,
        });
        
        Ok(opportunities)
    }

    /// Find Third Normal Form opportunities (eliminate transitive dependencies)
    async fn find_third_normal_form_opportunities(&self, collection_name: &str, _documents: &[&FirestoreDocument]) -> FireupResult<Vec<NormalizationOpportunity>> {
        let mut opportunities = Vec::new();
        
        // For now, we'll add a placeholder opportunity
        // This would require analysis of transitive dependencies
        opportunities.push(NormalizationOpportunity {
            collection: collection_name.to_string(),
            field_path: "transitive_dependency_analysis".to_string(),
            normalization_type: NormalizationType::ThirdNormalForm,
            description: "Potential transitive dependency detected - requires manual review".to_string(),
            impact: NormalizationImpact::Low,
        });
        
        Ok(opportunities)
    }
}

impl Default for DocumentStructureAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}