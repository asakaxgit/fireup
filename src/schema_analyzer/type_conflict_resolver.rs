use crate::error::FireupResult;
use crate::types::{
    TypeConflict, FieldTypeAnalysis, SchemaAnalysis
};
use std::collections::HashMap;
use tracing::{info, debug};

/// Type conflict resolver for handling data type inconsistencies
pub struct TypeConflictResolver {
    /// Minimum percentage for a type to be considered dominant
    dominant_type_threshold: f64,
    /// Minimum confidence for automatic resolution
    auto_resolution_confidence: f64,
}

impl TypeConflictResolver {
    /// Create a new type conflict resolver
    pub fn new() -> Self {
        Self {
            dominant_type_threshold: 0.7, // 70%
            auto_resolution_confidence: 0.8, // 80%
        }
    }

    /// Create a new type conflict resolver with custom thresholds
    pub fn with_thresholds(dominant_threshold: f64, auto_resolution_confidence: f64) -> Self {
        Self {
            dominant_type_threshold: dominant_threshold,
            auto_resolution_confidence,
        }
    }

    /// Detect and resolve type conflicts in schema analysis
    pub fn detect_and_resolve_conflicts(&self, analysis: &SchemaAnalysis) -> FireupResult<Vec<TypeConflict>> {
        info!("Detecting type conflicts across {} field types", analysis.field_types.len());
        
        let mut conflicts = Vec::new();
        
        for field_type in &analysis.field_types {
            if let Some(conflict) = self.analyze_field_for_conflicts(field_type)? {
                conflicts.push(conflict);
            }
        }
        
        info!("Detected {} type conflicts", conflicts.len());
        
        // Resolve conflicts where possible
        for conflict in &mut conflicts {
            self.resolve_conflict(conflict)?;
        }
        
        Ok(conflicts)
    }

    /// Analyze a single field for type conflicts
    fn analyze_field_for_conflicts(&self, field_type: &FieldTypeAnalysis) -> FireupResult<Option<TypeConflict>> {
        // Only consider it a conflict if there are multiple types
        if field_type.type_frequencies.len() <= 1 {
            return Ok(None);
        }
        
        let mut conflict = TypeConflict::new(field_type.field_path.clone());
        
        // Add all type occurrences
        for (type_name, count) in &field_type.type_frequencies {
            for _ in 0..*count {
                conflict.add_type_occurrence(type_name.clone());
            }
        }
        
        // Only return as conflict if no single type is overwhelmingly dominant
        let dominant_percentage = conflict.dominant_type_percentage();
        if dominant_percentage < (self.dominant_type_threshold * 100.0) {
            debug!("Type conflict detected for field '{}': {} types with max {}% dominance", 
                   field_type.field_path, field_type.type_frequencies.len(), dominant_percentage);
            Ok(Some(conflict))
        } else {
            Ok(None)
        }
    }

    /// Resolve a type conflict with suggestions
    fn resolve_conflict(&self, conflict: &mut TypeConflict) -> FireupResult<()> {
        let dominant_type = conflict.dominant_type().unwrap_or_default();
        let dominant_percentage = conflict.dominant_type_percentage();
        
        // Determine resolution strategy based on conflict analysis
        let (resolution, confidence) = self.determine_resolution_strategy(conflict)?;
        
        conflict.suggested_resolution = resolution;
        conflict.resolution_confidence = confidence;
        
        debug!("Resolved conflict for '{}': {} (confidence: {:.2})", 
               conflict.field_path, conflict.suggested_resolution, confidence);
        
        Ok(())
    }

    /// Determine the best resolution strategy for a type conflict
    fn determine_resolution_strategy(&self, conflict: &TypeConflict) -> FireupResult<(String, f64)> {
        let dominant_type = conflict.dominant_type().unwrap_or_default();
        let dominant_percentage = conflict.dominant_type_percentage();
        let type_count = conflict.conflicting_types.len();
        
        // Strategy 1: Use dominant type if it's reasonably common
        if dominant_percentage >= 60.0 {
            let confidence = (dominant_percentage / 100.0) * 0.9; // Scale down slightly
            return Ok((
                format!("Use dominant type '{}' ({:.1}% of occurrences). Consider data cleaning for remaining {:.1}% of values.", 
                        dominant_type, dominant_percentage, 100.0 - dominant_percentage),
                confidence
            ));
        }
        
        // Strategy 2: Use JSONB for highly mixed types
        if type_count > 3 || self.has_incompatible_types(conflict) {
            return Ok((
                "Use JSONB type to preserve all data variations. This maintains flexibility but may impact query performance.".to_string(),
                0.8
            ));
        }
        
        // Strategy 3: Use union type or TEXT for compatible types
        if self.are_types_compatible(conflict) {
            let compatible_type = self.find_compatible_type(conflict)?;
            return Ok((
                format!("Use '{}' type which can accommodate all variants. May require data conversion during import.", 
                        compatible_type),
                0.7
            ));
        }
        
        // Strategy 4: Manual review required
        Ok((
            "Manual review required. Consider splitting into multiple columns or using JSONB for complex cases.".to_string(),
            0.3
        ))
    }

    /// Check if the conflict contains incompatible types
    fn has_incompatible_types(&self, conflict: &TypeConflict) -> bool {
        let types: Vec<&String> = conflict.conflicting_types.iter().collect();
        
        // Check for fundamentally incompatible combinations
        let has_boolean = types.iter().any(|t| *t == "boolean");
        let has_array = types.iter().any(|t| *t == "array");
        let has_object = types.iter().any(|t| *t == "object");
        let has_numeric = types.iter().any(|t| *t == "integer" || *t == "number");
        
        // Boolean with anything else is usually incompatible
        if has_boolean && types.len() > 1 {
            return true;
        }
        
        // Arrays and objects with primitives are incompatible
        if (has_array || has_object) && (has_numeric || types.iter().any(|t| *t == "string")) {
            return true;
        }
        
        false
    }

    /// Check if types in the conflict are compatible
    fn are_types_compatible(&self, conflict: &TypeConflict) -> bool {
        let types: Vec<&String> = conflict.conflicting_types.iter().collect();
        
        // Numeric types are generally compatible
        let all_numeric = types.iter().all(|t| *t == "integer" || *t == "number");
        if all_numeric {
            return true;
        }
        
        // String-like types are compatible
        let all_string_like = types.iter().all(|t| {
            *t == "string" || *t == "uuid" || *t == "timestamp"
        });
        if all_string_like {
            return true;
        }
        
        // String can accommodate most primitive types
        if types.contains(&&"string".to_string()) && 
           types.iter().all(|t| *t != "array" && *t != "object") {
            return true;
        }
        
        false
    }

    /// Find a compatible type that can accommodate all variants
    fn find_compatible_type(&self, conflict: &TypeConflict) -> FireupResult<String> {
        let types: Vec<&String> = conflict.conflicting_types.iter().collect();
        
        // If all numeric, use NUMERIC
        if types.iter().all(|t| *t == "integer" || *t == "number") {
            return Ok("NUMERIC".to_string());
        }
        
        // If all string-like, use TEXT
        if types.iter().all(|t| {
            *t == "string" || *t == "uuid" || *t == "timestamp"
        }) {
            return Ok("TEXT".to_string());
        }
        
        // If string is present with primitives, use TEXT
        if types.contains(&&"string".to_string()) && 
           types.iter().all(|t| *t != "array" && *t != "object") {
            return Ok("TEXT".to_string());
        }
        
        // Default to JSONB for complex cases
        Ok("JSONB".to_string())
    }

    /// Generate detailed conflict report
    pub fn generate_conflict_report(&self, conflicts: &[TypeConflict]) -> String {
        let mut report = String::new();
        
        report.push_str("# Type Conflict Analysis Report\n\n");
        report.push_str(&format!("Total conflicts detected: {}\n\n", conflicts.len()));
        
        // Group conflicts by severity
        let high_severity: Vec<_> = conflicts.iter()
            .filter(|c| c.resolution_confidence < 0.5)
            .collect();
        let medium_severity: Vec<_> = conflicts.iter()
            .filter(|c| c.resolution_confidence >= 0.5 && c.resolution_confidence < 0.8)
            .collect();
        let low_severity: Vec<_> = conflicts.iter()
            .filter(|c| c.resolution_confidence >= 0.8)
            .collect();
        
        if !high_severity.is_empty() {
            report.push_str("## High Severity Conflicts (Manual Review Required)\n\n");
            for conflict in high_severity {
                report.push_str(&self.format_conflict_details(conflict));
                report.push_str("\n");
            }
        }
        
        if !medium_severity.is_empty() {
            report.push_str("## Medium Severity Conflicts (Review Recommended)\n\n");
            for conflict in medium_severity {
                report.push_str(&self.format_conflict_details(conflict));
                report.push_str("\n");
            }
        }
        
        if !low_severity.is_empty() {
            report.push_str("## Low Severity Conflicts (Auto-Resolvable)\n\n");
            for conflict in low_severity {
                report.push_str(&self.format_conflict_details(conflict));
                report.push_str("\n");
            }
        }
        
        report
    }

    /// Format conflict details for reporting
    fn format_conflict_details(&self, conflict: &TypeConflict) -> String {
        let mut details = String::new();
        
        details.push_str(&format!("### Field: `{}`\n", conflict.field_path));
        details.push_str(&format!("**Total Occurrences:** {}\n", conflict.total_occurrences));
        details.push_str("**Type Distribution:**\n");
        
        // Sort types by occurrence count
        let mut sorted_types: Vec<_> = conflict.type_occurrences.iter().collect();
        sorted_types.sort_by(|a, b| b.1.cmp(a.1));
        
        for (type_name, count) in sorted_types {
            let percentage = (*count as f64 / conflict.total_occurrences as f64) * 100.0;
            details.push_str(&format!("- {}: {} occurrences ({:.1}%)\n", type_name, count, percentage));
        }
        
        details.push_str(&format!("**Suggested Resolution:** {}\n", conflict.suggested_resolution));
        details.push_str(&format!("**Confidence:** {:.1}%\n", conflict.resolution_confidence * 100.0));
        
        details
    }

    /// Get statistics about type conflicts
    pub fn get_conflict_statistics(&self, conflicts: &[TypeConflict]) -> ConflictStatistics {
        let total_conflicts = conflicts.len();
        let auto_resolvable = conflicts.iter()
            .filter(|c| c.resolution_confidence >= self.auto_resolution_confidence)
            .count();
        let manual_review_required = conflicts.iter()
            .filter(|c| c.resolution_confidence < 0.5)
            .count();
        
        let total_affected_fields = conflicts.iter()
            .map(|c| c.total_occurrences)
            .sum::<u32>();
        
        let most_common_conflict_types = self.analyze_common_conflict_patterns(conflicts);
        
        ConflictStatistics {
            total_conflicts,
            auto_resolvable,
            manual_review_required,
            total_affected_fields,
            most_common_conflict_types,
        }
    }

    /// Analyze common patterns in type conflicts
    fn analyze_common_conflict_patterns(&self, conflicts: &[TypeConflict]) -> Vec<(String, usize)> {
        let mut pattern_counts = HashMap::new();
        
        for conflict in conflicts {
            let mut types = conflict.conflicting_types.clone();
            types.sort();
            let pattern = types.join(" + ");
            
            *pattern_counts.entry(pattern).or_insert(0) += 1;
        }
        
        let mut patterns: Vec<_> = pattern_counts.into_iter().collect();
        patterns.sort_by(|a, b| b.1.cmp(&a.1));
        patterns.truncate(5); // Top 5 patterns
        
        patterns
    }
}

/// Statistics about type conflicts
#[derive(Debug, Clone)]
pub struct ConflictStatistics {
    pub total_conflicts: usize,
    pub auto_resolvable: usize,
    pub manual_review_required: usize,
    pub total_affected_fields: u32,
    pub most_common_conflict_types: Vec<(String, usize)>,
}

impl Default for TypeConflictResolver {
    fn default() -> Self {
        Self::new()
    }
}