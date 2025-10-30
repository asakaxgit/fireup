use crate::error::FireupError;
use crate::types::PostgreSQLType;
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;
use chrono::DateTime;

/// Maps Firestore data types to appropriate PostgreSQL types
pub struct DataTypeMapper {
    /// Custom type mappings for specific field paths
    custom_mappings: HashMap<String, PostgreSQLType>,
    /// Configuration for array handling
    array_config: ArrayHandlingConfig,
}

/// Configuration for how arrays should be handled during type mapping
#[derive(Debug, Clone)]
pub struct ArrayHandlingConfig {
    /// Maximum array size before creating separate table
    max_inline_array_size: usize,
    /// Whether to use JSONB for complex arrays
    use_jsonb_for_complex: bool,
    /// Whether to normalize arrays into separate tables
    normalize_arrays: bool,
}

/// Result of type mapping operation
#[derive(Debug, Clone)]
pub struct TypeMappingResult {
    /// The mapped PostgreSQL type
    pub postgres_type: PostgreSQLType,
    /// Whether this field should be normalized to a separate table
    pub requires_normalization: bool,
    /// Additional metadata about the mapping
    pub metadata: TypeMappingMetadata,
}

/// Metadata about a type mapping decision
#[derive(Debug, Clone)]
pub struct TypeMappingMetadata {
    /// Original Firestore type detected
    pub original_type: String,
    /// Confidence in the mapping (0.0 to 1.0)
    pub confidence: f64,
    /// Warnings about the mapping
    pub warnings: Vec<String>,
    /// Whether the type was inferred from multiple samples
    pub inferred: bool,
}

impl Default for ArrayHandlingConfig {
    fn default() -> Self {
        Self {
            max_inline_array_size: 10,
            use_jsonb_for_complex: true,
            normalize_arrays: true,
        }
    }
}

impl DataTypeMapper {
    /// Create a new data type mapper with default configuration
    pub fn new() -> Self {
        Self {
            custom_mappings: HashMap::new(),
            array_config: ArrayHandlingConfig::default(),
        }
    }

    /// Create a new data type mapper with custom configuration
    pub fn with_config(array_config: ArrayHandlingConfig) -> Self {
        Self {
            custom_mappings: HashMap::new(),
            array_config,
        }
    }

    /// Add a custom type mapping for a specific field path
    pub fn add_custom_mapping(&mut self, field_path: String, postgres_type: PostgreSQLType) {
        self.custom_mappings.insert(field_path, postgres_type);
    }

    /// Map a Firestore value to a PostgreSQL type
    pub fn map_value_type(&self, value: &Value, field_path: &str) -> Result<TypeMappingResult, FireupError> {
        // Check for custom mappings first
        if let Some(custom_type) = self.custom_mappings.get(field_path) {
            return Ok(TypeMappingResult {
                postgres_type: custom_type.clone(),
                requires_normalization: false,
                metadata: TypeMappingMetadata {
                    original_type: self.detect_firestore_type(value),
                    confidence: 1.0,
                    warnings: vec!["Using custom type mapping".to_string()],
                    inferred: false,
                },
            });
        }

        self.map_value_type_internal(value, field_path)
    }

    /// Internal method to map value types
    fn map_value_type_internal(&self, value: &Value, field_path: &str) -> Result<TypeMappingResult, FireupError> {
        match value {
            Value::Null => Ok(TypeMappingResult {
                postgres_type: PostgreSQLType::Text,
                requires_normalization: false,
                metadata: TypeMappingMetadata {
                    original_type: "null".to_string(),
                    confidence: 0.5,
                    warnings: vec!["Null value mapped to TEXT, consider making nullable".to_string()],
                    inferred: true,
                },
            }),

            Value::Bool(_) => Ok(TypeMappingResult {
                postgres_type: PostgreSQLType::Boolean,
                requires_normalization: false,
                metadata: TypeMappingMetadata {
                    original_type: "boolean".to_string(),
                    confidence: 1.0,
                    warnings: vec![],
                    inferred: false,
                },
            }),

            Value::Number(n) => {
                if n.is_i64() {
                    let int_val = n.as_i64().unwrap();
                    if int_val >= i32::MIN as i64 && int_val <= i32::MAX as i64 {
                        Ok(TypeMappingResult {
                            postgres_type: PostgreSQLType::Integer,
                            requires_normalization: false,
                            metadata: TypeMappingMetadata {
                                original_type: "integer".to_string(),
                                confidence: 1.0,
                                warnings: vec![],
                                inferred: false,
                            },
                        })
                    } else {
                        Ok(TypeMappingResult {
                            postgres_type: PostgreSQLType::BigInt,
                            requires_normalization: false,
                            metadata: TypeMappingMetadata {
                                original_type: "bigint".to_string(),
                                confidence: 1.0,
                                warnings: vec![],
                                inferred: false,
                            },
                        })
                    }
                } else if n.is_f64() {
                    Ok(TypeMappingResult {
                        postgres_type: PostgreSQLType::Numeric(Some(15), Some(6)),
                        requires_normalization: false,
                        metadata: TypeMappingMetadata {
                            original_type: "float".to_string(),
                            confidence: 1.0,
                            warnings: vec![],
                            inferred: false,
                        },
                    })
                } else {
                    Ok(TypeMappingResult {
                        postgres_type: PostgreSQLType::Numeric(None, None),
                        requires_normalization: false,
                        metadata: TypeMappingMetadata {
                            original_type: "number".to_string(),
                            confidence: 0.8,
                            warnings: vec!["Unknown number format, using generic NUMERIC".to_string()],
                            inferred: true,
                        },
                    })
                }
            },

            Value::String(s) => {
                // Check for special string formats
                if self.is_uuid_string(s) {
                    Ok(TypeMappingResult {
                        postgres_type: PostgreSQLType::Uuid,
                        requires_normalization: false,
                        metadata: TypeMappingMetadata {
                            original_type: "uuid_string".to_string(),
                            confidence: 0.9,
                            warnings: vec![],
                            inferred: true,
                        },
                    })
                } else if self.is_timestamp_string(s) {
                    Ok(TypeMappingResult {
                        postgres_type: PostgreSQLType::Timestamp,
                        requires_normalization: false,
                        metadata: TypeMappingMetadata {
                            original_type: "timestamp_string".to_string(),
                            confidence: 0.9,
                            warnings: vec!["String appears to be timestamp, consider parsing".to_string()],
                            inferred: true,
                        },
                    })
                } else if self.is_reference_string(s) {
                    Ok(TypeMappingResult {
                        postgres_type: PostgreSQLType::Uuid,
                        requires_normalization: false,
                        metadata: TypeMappingMetadata {
                            original_type: "reference".to_string(),
                            confidence: 0.8,
                            warnings: vec!["Firestore reference mapped to UUID".to_string()],
                            inferred: true,
                        },
                    })
                } else {
                    // Regular string - determine appropriate VARCHAR length or use TEXT
                    let postgres_type = if s.len() <= 255 {
                        PostgreSQLType::Varchar(Some(255))
                    } else if s.len() <= 1000 {
                        PostgreSQLType::Varchar(Some(1000))
                    } else {
                        PostgreSQLType::Text
                    };

                    Ok(TypeMappingResult {
                        postgres_type,
                        requires_normalization: false,
                        metadata: TypeMappingMetadata {
                            original_type: "string".to_string(),
                            confidence: 1.0,
                            warnings: vec![],
                            inferred: false,
                        },
                    })
                }
            },

            Value::Array(arr) => {
                if arr.is_empty() {
                    return Ok(TypeMappingResult {
                        postgres_type: PostgreSQLType::Jsonb,
                        requires_normalization: false,
                        metadata: TypeMappingMetadata {
                            original_type: "empty_array".to_string(),
                            confidence: 0.5,
                            warnings: vec!["Empty array mapped to JSONB".to_string()],
                            inferred: true,
                        },
                    });
                }

                // Check if array should be normalized
                let should_normalize = self.array_config.normalize_arrays && 
                    arr.len() > self.array_config.max_inline_array_size;

                if should_normalize {
                    return Ok(TypeMappingResult {
                        postgres_type: PostgreSQLType::Text, // Placeholder - will be handled by normalization
                        requires_normalization: true,
                        metadata: TypeMappingMetadata {
                            original_type: "large_array".to_string(),
                            confidence: 1.0,
                            warnings: vec!["Array will be normalized to separate table".to_string()],
                            inferred: false,
                        },
                    });
                }

                // Analyze array element types
                let element_types = self.analyze_array_element_types(arr, field_path)?;
                
                if element_types.len() == 1 {
                    // Homogeneous array - create typed array
                    let element_type = element_types.into_iter().next().unwrap();
                    Ok(TypeMappingResult {
                        postgres_type: PostgreSQLType::Array(Box::new(element_type.postgres_type)),
                        requires_normalization: false,
                        metadata: TypeMappingMetadata {
                            original_type: "homogeneous_array".to_string(),
                            confidence: 0.9,
                            warnings: vec![],
                            inferred: false,
                        },
                    })
                } else if self.array_config.use_jsonb_for_complex {
                    // Heterogeneous array - use JSONB
                    Ok(TypeMappingResult {
                        postgres_type: PostgreSQLType::Jsonb,
                        requires_normalization: false,
                        metadata: TypeMappingMetadata {
                            original_type: "heterogeneous_array".to_string(),
                            confidence: 0.8,
                            warnings: vec!["Mixed-type array stored as JSONB".to_string()],
                            inferred: true,
                        },
                    })
                } else {
                    // Normalize heterogeneous array
                    Ok(TypeMappingResult {
                        postgres_type: PostgreSQLType::Text, // Placeholder
                        requires_normalization: true,
                        metadata: TypeMappingMetadata {
                            original_type: "heterogeneous_array".to_string(),
                            confidence: 1.0,
                            warnings: vec!["Mixed-type array will be normalized".to_string()],
                            inferred: false,
                        },
                    })
                }
            },

            Value::Object(obj) => {
                if obj.is_empty() {
                    return Ok(TypeMappingResult {
                        postgres_type: PostgreSQLType::Jsonb,
                        requires_normalization: false,
                        metadata: TypeMappingMetadata {
                            original_type: "empty_object".to_string(),
                            confidence: 0.5,
                            warnings: vec!["Empty object mapped to JSONB".to_string()],
                            inferred: true,
                        },
                    });
                }

                // Check if object should be normalized based on complexity
                let should_normalize = self.should_normalize_object(obj);

                if should_normalize {
                    Ok(TypeMappingResult {
                        postgres_type: PostgreSQLType::Text, // Placeholder
                        requires_normalization: true,
                        metadata: TypeMappingMetadata {
                            original_type: "complex_object".to_string(),
                            confidence: 1.0,
                            warnings: vec!["Complex object will be normalized to separate table".to_string()],
                            inferred: false,
                        },
                    })
                } else {
                    Ok(TypeMappingResult {
                        postgres_type: PostgreSQLType::Jsonb,
                        requires_normalization: false,
                        metadata: TypeMappingMetadata {
                            original_type: "simple_object".to_string(),
                            confidence: 0.9,
                            warnings: vec!["Object stored as JSONB".to_string()],
                            inferred: false,
                        },
                    })
                }
            },
        }
    }

    /// Analyze the types of elements in an array
    fn analyze_array_element_types(&self, arr: &[Value], field_path: &str) -> Result<Vec<TypeMappingResult>, FireupError> {
        let mut unique_types = HashMap::new();
        
        for (i, element) in arr.iter().enumerate() {
            let element_path = format!("{}[{}]", field_path, i);
            let mapping = self.map_value_type_internal(element, &element_path)?;
            let type_key = format!("{:?}", mapping.postgres_type);
            unique_types.insert(type_key, mapping);
        }

        Ok(unique_types.into_values().collect())
    }

    /// Determine if an object should be normalized to a separate table
    fn should_normalize_object(&self, obj: &serde_json::Map<String, Value>) -> bool {
        // Normalize if object has more than 3 fields or contains nested objects/arrays
        if obj.len() > 3 {
            return true;
        }

        for value in obj.values() {
            match value {
                Value::Object(_) | Value::Array(_) => return true,
                _ => continue,
            }
        }

        false
    }

    /// Check if a string looks like a UUID
    fn is_uuid_string(&self, s: &str) -> bool {
        Uuid::parse_str(s).is_ok()
    }

    /// Check if a string looks like a timestamp
    fn is_timestamp_string(&self, s: &str) -> bool {
        // Try parsing common timestamp formats
        DateTime::parse_from_rfc3339(s).is_ok() ||
        DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").is_ok() ||
        DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.fZ").is_ok()
    }

    /// Check if a string looks like a Firestore reference
    fn is_reference_string(&self, s: &str) -> bool {
        // Firestore references typically follow pattern: projects/{project}/databases/{database}/documents/{collection}/{document}
        s.starts_with("projects/") && s.contains("/databases/") && s.contains("/documents/")
    }

    /// Detect the Firestore type name for a value
    fn detect_firestore_type(&self, value: &Value) -> String {
        match value {
            Value::Null => "null".to_string(),
            Value::Bool(_) => "boolean".to_string(),
            Value::Number(n) => {
                if n.is_i64() {
                    "integer".to_string()
                } else {
                    "number".to_string()
                }
            },
            Value::String(_) => "string".to_string(),
            Value::Array(_) => "array".to_string(),
            Value::Object(_) => "map".to_string(),
        }
    }

    /// Map multiple values to determine the best common type
    pub fn map_multiple_values(&self, values: &[&Value], field_path: &str) -> Result<TypeMappingResult, FireupError> {
        if values.is_empty() {
            return Err(FireupError::TypeMapping("No values provided for type mapping".to_string()));
        }

        if values.len() == 1 {
            return self.map_value_type(values[0], field_path);
        }

        // Analyze all values and find the most compatible type
        let mut type_mappings = Vec::new();
        for (i, value) in values.iter().enumerate() {
            let value_path = format!("{}[sample_{}]", field_path, i);
            let mapping = self.map_value_type(value, &value_path)?;
            type_mappings.push(mapping);
        }

        // Find the most compatible type
        self.find_compatible_type(type_mappings, field_path)
    }

    /// Find a compatible PostgreSQL type for multiple type mappings
    fn find_compatible_type(&self, mappings: Vec<TypeMappingResult>, field_path: &str) -> Result<TypeMappingResult, FireupError> {
        if mappings.is_empty() {
            return Err(FireupError::TypeMapping("No type mappings provided".to_string()));
        }

        if mappings.len() == 1 {
            return Ok(mappings.into_iter().next().unwrap());
        }

        // Check if all mappings have the same type
        let first_type = &mappings[0].postgres_type;
        if mappings.iter().all(|m| std::mem::discriminant(&m.postgres_type) == std::mem::discriminant(first_type)) {
            return Ok(mappings.into_iter().next().unwrap());
        }

        // Handle type conflicts by finding the most general compatible type
        let mut warnings = Vec::new();
        let mut requires_normalization = false;

        // Check for normalization requirements
        if mappings.iter().any(|m| m.requires_normalization) {
            requires_normalization = true;
        }

        // Determine the most general type
        let postgres_type = if mappings.iter().any(|m| matches!(m.postgres_type, PostgreSQLType::Text)) {
            warnings.push("Type conflict resolved by using TEXT".to_string());
            PostgreSQLType::Text
        } else if mappings.iter().any(|m| matches!(m.postgres_type, PostgreSQLType::Jsonb)) {
            warnings.push("Type conflict resolved by using JSONB".to_string());
            PostgreSQLType::Jsonb
        } else if mappings.iter().all(|m| matches!(m.postgres_type, PostgreSQLType::Varchar(_) | PostgreSQLType::Text)) {
            warnings.push("String types unified to TEXT".to_string());
            PostgreSQLType::Text
        } else if mappings.iter().all(|m| matches!(m.postgres_type, PostgreSQLType::Integer | PostgreSQLType::BigInt | PostgreSQLType::Numeric(_, _))) {
            warnings.push("Numeric types unified to NUMERIC".to_string());
            PostgreSQLType::Numeric(None, None)
        } else {
            warnings.push("Incompatible types resolved by using JSONB".to_string());
            PostgreSQLType::Jsonb
        };

        Ok(TypeMappingResult {
            postgres_type,
            requires_normalization,
            metadata: TypeMappingMetadata {
                original_type: "mixed".to_string(),
                confidence: 0.6,
                warnings,
                inferred: true,
            },
        })
    }
}

impl Default for DataTypeMapper {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_map_basic_types() {
        let mapper = DataTypeMapper::new();

        // Test boolean
        let result = mapper.map_value_type(&json!(true), "test.bool").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Boolean));

        // Test integer
        let result = mapper.map_value_type(&json!(42), "test.int").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Integer));

        // Test string
        let result = mapper.map_value_type(&json!("hello"), "test.string").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Varchar(_)));
    }

    #[test]
    fn test_map_uuid_string() {
        let mapper = DataTypeMapper::new();
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        
        let result = mapper.map_value_type(&json!(uuid_str), "test.uuid").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Uuid));
    }

    #[test]
    fn test_map_array() {
        let mapper = DataTypeMapper::new();
        
        // Homogeneous array
        let result = mapper.map_value_type(&json!([1, 2, 3]), "test.array").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Array(_)));

        // Heterogeneous array
        let result = mapper.map_value_type(&json!([1, "hello", true]), "test.mixed_array").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Jsonb));
    }

    #[test]
    fn test_map_object() {
        let mapper = DataTypeMapper::new();
        
        // Simple object
        let result = mapper.map_value_type(&json!({"name": "test"}), "test.simple").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Jsonb));
        assert!(!result.requires_normalization);

        // Complex object (should be normalized)
        let complex_obj = json!({
            "field1": "value1",
            "field2": "value2", 
            "field3": "value3",
            "field4": "value4",
            "nested": {"inner": "value"}
        });
        let result = mapper.map_value_type(&complex_obj, "test.complex").unwrap();
        assert!(result.requires_normalization);
    }

    #[test]
    fn test_custom_mappings() {
        let mut mapper = DataTypeMapper::new();
        mapper.add_custom_mapping("user.id".to_string(), PostgreSQLType::Uuid);

        let result = mapper.map_value_type(&json!("some-string"), "user.id").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Uuid));
    }

    #[test]
    fn test_multiple_values_mapping() {
        let mapper = DataTypeMapper::new();
        let val1 = json!(42);
        let val2 = json!(100);
        let val3 = json!(999);
        let values = vec![&val1, &val2, &val3];

        let result = mapper.map_multiple_values(&values, "test.numbers").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Integer));
    }

    #[test]
    fn test_type_conflict_resolution() {
        let mapper = DataTypeMapper::new();
        let val1 = json!(42);
        let val2 = json!("hello");
        let val3 = json!(true);
        let values = vec![&val1, &val2, &val3];

        let result = mapper.map_multiple_values(&values, "test.mixed").unwrap();
        assert!(matches!(result.postgres_type, PostgreSQLType::Jsonb));
        assert!(!result.metadata.warnings.is_empty());
    }
}