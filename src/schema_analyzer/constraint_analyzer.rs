use crate::error::{FireupError, FireupResult};
use crate::types::{
    FirestoreDocument, SchemaAnalysis, FieldTypeAnalysis, Constraint, ConstraintType
};
use std::collections::{HashMap, HashSet};
use serde_json::Value;
use tracing::{info, debug, warn};

/// Constraint analyzer for determining column constraints
pub struct ConstraintAnalyzer {
    /// Minimum percentage for NOT NULL recommendation
    not_null_threshold: f64,
    /// Minimum percentage for UNIQUE recommendation
    unique_threshold: f64,
    /// Minimum sample size for reliable analysis
    min_sample_size: usize,
}

impl ConstraintAnalyzer {
    /// Create a new constraint analyzer
    pub fn new() -> Self {
        Self {
            not_null_threshold: 0.95, // 95% presence required for NOT NULL
            unique_threshold: 0.98,   // 98% uniqueness required for UNIQUE
            min_sample_size: 10,      // Minimum 10 documents for analysis
        }
    }

    /// Create a new constraint analyzer with custom thresholds
    pub fn with_thresholds(not_null_threshold: f64, unique_threshold: f64, min_sample_size: usize) -> Self {
        Self {
            not_null_threshold,
            unique_threshold,
            min_sample_size,
        }
    }

    /// Analyze field completeness and recommend constraints
    pub fn analyze_constraints(&self, documents: &[FirestoreDocument], analysis: &SchemaAnalysis) -> FireupResult<Vec<Constraint>> {
        info!("Analyzing constraints for {} documents across {} collections", 
              documents.len(), analysis.collections.len());
        
        let mut constraints = Vec::new();
        
        // Group documents by collection for analysis
        let collections = self.group_documents_by_collection(documents);
        
        for (collection_name, collection_docs) in collections {
            if collection_docs.len() < self.min_sample_size {
                warn!("Skipping constraint analysis for collection '{}' - insufficient sample size ({} < {})", 
                      collection_name, collection_docs.len(), self.min_sample_size);
                continue;
            }
            
            debug!("Analyzing constraints for collection '{}' with {} documents", 
                   collection_name, collection_docs.len());
            
            // Analyze NOT NULL constraints
            let not_null_constraints = self.analyze_not_null_constraints(&collection_name, &collection_docs, analysis)?;
            constraints.extend(not_null_constraints);
            
            // Analyze UNIQUE constraints
            let unique_constraints = self.analyze_unique_constraints(&collection_name, &collection_docs)?;
            constraints.extend(unique_constraints);
            
            // Analyze CHECK constraints
            let check_constraints = self.analyze_check_constraints(&collection_name, &collection_docs)?;
            constraints.extend(check_constraints);
        }
        
        info!("Generated {} constraint recommendations", constraints.len());
        Ok(constraints)
    }

    /// Group documents by collection name
    fn group_documents_by_collection<'a>(&self, documents: &'a [FirestoreDocument]) -> HashMap<String, Vec<&'a FirestoreDocument>> {
        let mut collections = HashMap::new();
        
        for doc in documents {
            collections.entry(doc.collection.clone())
                .or_insert_with(Vec::new)
                .push(doc);
        }
        
        collections
    }

    /// Analyze NOT NULL constraint opportunities
    fn analyze_not_null_constraints(
        &self, 
        collection_name: &str, 
        documents: &[&FirestoreDocument],
        analysis: &SchemaAnalysis
    ) -> FireupResult<Vec<Constraint>> {
        let mut constraints = Vec::new();
        
        // Get field type analysis for this collection
        let collection_fields: Vec<_> = analysis.field_types.iter()
            .filter(|ft| ft.field_path.starts_with(&format!("{}.", collection_name)))
            .collect();
        
        for field_type in collection_fields {
            let field_name = field_type.field_path
                .strip_prefix(&format!("{}.", collection_name))
                .unwrap_or(&field_type.field_path)
                .replace('.', "_"); // Handle nested fields
            
            // Recommend NOT NULL if field is present in enough documents
            if field_type.presence_percentage >= (self.not_null_threshold * 100.0) {
                debug!("Recommending NOT NULL for field '{}' ({}% presence)", 
                       field_name, field_type.presence_percentage);
                
                constraints.push(Constraint {
                    name: format!("nn_{}_{}", collection_name, field_name),
                    table: collection_name.to_string(),
                    constraint_type: ConstraintType::NotNull,
                    columns: vec![field_name],
                    parameters: HashMap::new(),
                });
            }
        }
        
        Ok(constraints)
    }

    /// Analyze UNIQUE constraint opportunities
    fn analyze_unique_constraints(&self, collection_name: &str, documents: &[&FirestoreDocument]) -> FireupResult<Vec<Constraint>> {
        let mut constraints = Vec::new();
        let mut field_values: HashMap<String, HashSet<String>> = HashMap::new();
        
        // Collect all field values
        for doc in documents {
            self.collect_field_values(&mut field_values, &doc.data, "")?;
        }
        
        // Analyze uniqueness for each field
        for (field_path, values) in field_values {
            let field_name = field_path.replace('.', "_");
            let uniqueness_ratio = values.len() as f64 / documents.len() as f64;
            
            // Recommend UNIQUE if values are sufficiently unique
            if uniqueness_ratio >= self.unique_threshold {
                debug!("Recommending UNIQUE for field '{}' ({:.1}% unique values)", 
                       field_name, uniqueness_ratio * 100.0);
                
                constraints.push(Constraint {
                    name: format!("uq_{}_{}", collection_name, field_name),
                    table: collection_name.to_string(),
                    constraint_type: ConstraintType::Unique,
                    columns: vec![field_name],
                    parameters: HashMap::new(),
                });
            }
        }
        
        Ok(constraints)
    }

    /// Collect field values for uniqueness analysis
    fn collect_field_values(
        &self,
        field_values: &mut HashMap<String, HashSet<String>>,
        data: &HashMap<String, Value>,
        parent_path: &str,
    ) -> FireupResult<()> {
        for (key, value) in data {
            let field_path = if parent_path.is_empty() {
                key.clone()
            } else {
                format!("{}.{}", parent_path, key)
            };
            
            // Convert value to string for uniqueness analysis
            let value_str = match value {
                Value::Null => continue, // Skip null values
                Value::Bool(b) => b.to_string(),
                Value::Number(n) => n.to_string(),
                Value::String(s) => s.clone(),
                Value::Array(_) => continue, // Skip arrays for uniqueness
                Value::Object(nested_obj) => {
                    // Recursively analyze nested objects
                    // Convert serde_json::Map to HashMap for recursive call
                    let nested_hashmap: HashMap<String, Value> = nested_obj.iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    self.collect_field_values(field_values, &nested_hashmap, &field_path)?;
                    continue;
                }
            };
            
            field_values.entry(field_path)
                .or_insert_with(HashSet::new)
                .insert(value_str);
        }
        
        Ok(())
    }

    /// Analyze CHECK constraint opportunities
    fn analyze_check_constraints(&self, collection_name: &str, documents: &[&FirestoreDocument]) -> FireupResult<Vec<Constraint>> {
        let mut constraints = Vec::new();
        let mut field_ranges: HashMap<String, FieldRange> = HashMap::new();
        
        // Analyze numeric field ranges
        for doc in documents {
            self.analyze_field_ranges(&mut field_ranges, &doc.data, "")?;
        }
        
        // Generate CHECK constraints for numeric ranges
        for (field_path, range) in field_ranges {
            let field_name = field_path.replace('.', "_");
            
            if let Some(check_constraint) = self.generate_range_check_constraint(collection_name, &field_name, &range)? {
                constraints.push(check_constraint);
            }
        }
        
        Ok(constraints)
    }

    /// Analyze numeric field ranges for CHECK constraints
    fn analyze_field_ranges(
        &self,
        field_ranges: &mut HashMap<String, FieldRange>,
        data: &HashMap<String, Value>,
        parent_path: &str,
    ) -> FireupResult<()> {
        for (key, value) in data {
            let field_path = if parent_path.is_empty() {
                key.clone()
            } else {
                format!("{}.{}", parent_path, key)
            };
            
            match value {
                Value::Number(n) => {
                    let range = field_ranges.entry(field_path).or_insert_with(FieldRange::new);
                    
                    if let Some(f) = n.as_f64() {
                        range.update(f);
                    }
                }
                Value::Object(nested_obj) => {
                    // Convert serde_json::Map to HashMap for recursive call
                    let nested_hashmap: HashMap<String, Value> = nested_obj.iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    self.analyze_field_ranges(field_ranges, &nested_hashmap, &field_path)?;
                }
                _ => {} // Skip non-numeric values
            }
        }
        
        Ok(())
    }

    /// Generate a CHECK constraint for numeric ranges
    fn generate_range_check_constraint(
        &self,
        collection_name: &str,
        field_name: &str,
        range: &FieldRange,
    ) -> FireupResult<Option<Constraint>> {
        // Only generate CHECK constraints for reasonable ranges
        if range.sample_count < self.min_sample_size {
            return Ok(None);
        }
        
        // Generate constraint for positive values if all values are positive
        if range.min_value >= 0.0 {
            let mut parameters = HashMap::new();
            parameters.insert("condition".to_string(), format!("{} >= 0", field_name));
            
            debug!("Recommending CHECK constraint for field '{}' (all values >= 0)", field_name);
            
            return Ok(Some(Constraint {
                name: format!("chk_{}_{}_positive", collection_name, field_name),
                table: collection_name.to_string(),
                constraint_type: ConstraintType::Check,
                columns: vec![field_name.to_string()],
                parameters,
            }));
        }
        
        // Generate constraint for reasonable ranges (avoid extreme outliers)
        let range_size = range.max_value - range.min_value;
        if range_size > 0.0 && range_size < 1_000_000.0 { // Reasonable range
            let mut parameters = HashMap::new();
            parameters.insert(
                "condition".to_string(), 
                format!("{} BETWEEN {} AND {}", field_name, range.min_value, range.max_value)
            );
            
            debug!("Recommending CHECK constraint for field '{}' (range: {} to {})", 
                   field_name, range.min_value, range.max_value);
            
            return Ok(Some(Constraint {
                name: format!("chk_{}_{}_range", collection_name, field_name),
                table: collection_name.to_string(),
                constraint_type: ConstraintType::Check,
                columns: vec![field_name.to_string()],
                parameters,
            }));
        }
        
        Ok(None)
    }

    /// Generate constraint analysis report
    pub fn generate_constraint_report(&self, constraints: &[Constraint]) -> String {
        let mut report = String::new();
        
        report.push_str("# Constraint Analysis Report\n\n");
        report.push_str(&format!("Total constraints recommended: {}\n\n", constraints.len()));
        
        // Group constraints by type
        let not_null_constraints: Vec<_> = constraints.iter()
            .filter(|c| matches!(c.constraint_type, ConstraintType::NotNull))
            .collect();
        let unique_constraints: Vec<_> = constraints.iter()
            .filter(|c| matches!(c.constraint_type, ConstraintType::Unique))
            .collect();
        let check_constraints: Vec<_> = constraints.iter()
            .filter(|c| matches!(c.constraint_type, ConstraintType::Check))
            .collect();
        
        if !not_null_constraints.is_empty() {
            report.push_str("## NOT NULL Constraints\n\n");
            report.push_str(&format!("Recommended {} NOT NULL constraints for fields with high presence rates.\n\n", not_null_constraints.len()));
            for constraint in not_null_constraints {
                report.push_str(&format!("- `{}`.`{}`: {}\n", 
                                       constraint.table, 
                                       constraint.columns.join(", "),
                                       constraint.name));
            }
            report.push_str("\n");
        }
        
        if !unique_constraints.is_empty() {
            report.push_str("## UNIQUE Constraints\n\n");
            report.push_str(&format!("Recommended {} UNIQUE constraints for fields with high uniqueness.\n\n", unique_constraints.len()));
            for constraint in unique_constraints {
                report.push_str(&format!("- `{}`.`{}`: {}\n", 
                                       constraint.table, 
                                       constraint.columns.join(", "),
                                       constraint.name));
            }
            report.push_str("\n");
        }
        
        if !check_constraints.is_empty() {
            report.push_str("## CHECK Constraints\n\n");
            report.push_str(&format!("Recommended {} CHECK constraints for data validation.\n\n", check_constraints.len()));
            for constraint in check_constraints {
                let default_condition = "N/A".to_string();
                let condition = constraint.parameters.get("condition").unwrap_or(&default_condition);
                report.push_str(&format!("- `{}`.`{}`: {} ({})\n", 
                                       constraint.table, 
                                       constraint.columns.join(", "),
                                       constraint.name,
                                       condition));
            }
            report.push_str("\n");
        }
        
        report
    }

    /// Get constraint statistics
    pub fn get_constraint_statistics(&self, constraints: &[Constraint]) -> ConstraintStatistics {
        let total_constraints = constraints.len();
        let not_null_count = constraints.iter()
            .filter(|c| matches!(c.constraint_type, ConstraintType::NotNull))
            .count();
        let unique_count = constraints.iter()
            .filter(|c| matches!(c.constraint_type, ConstraintType::Unique))
            .count();
        let check_count = constraints.iter()
            .filter(|c| matches!(c.constraint_type, ConstraintType::Check))
            .count();
        
        // Count constraints by table
        let mut table_counts = HashMap::new();
        for constraint in constraints {
            *table_counts.entry(constraint.table.clone()).or_insert(0) += 1;
        }
        
        ConstraintStatistics {
            total_constraints,
            not_null_count,
            unique_count,
            check_count,
            constraints_by_table: table_counts,
        }
    }
}

/// Field range information for CHECK constraint analysis
#[derive(Debug, Clone)]
struct FieldRange {
    min_value: f64,
    max_value: f64,
    sample_count: usize,
}

impl FieldRange {
    fn new() -> Self {
        Self {
            min_value: f64::INFINITY,
            max_value: f64::NEG_INFINITY,
            sample_count: 0,
        }
    }
    
    fn update(&mut self, value: f64) {
        if value < self.min_value {
            self.min_value = value;
        }
        if value > self.max_value {
            self.max_value = value;
        }
        self.sample_count += 1;
    }
}

/// Statistics about constraint analysis
#[derive(Debug, Clone)]
pub struct ConstraintStatistics {
    pub total_constraints: usize,
    pub not_null_count: usize,
    pub unique_count: usize,
    pub check_count: usize,
    pub constraints_by_table: HashMap<String, usize>,
}

impl Default for ConstraintAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}