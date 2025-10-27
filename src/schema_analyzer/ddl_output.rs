use crate::types::{
    NormalizedSchema, SchemaAnalysis, WarningLevel
};
use crate::schema_analyzer::{
    DDLGenerator, GeneratedDDL, ConstraintGenerator, IndexGenerator,
    ConstraintAnalysisResult, IndexAnalysisResult
};
use crate::error::{FireupResult, FireupError};
use std::fs;
use chrono::Utc;

/// DDL file output manager for generating review files
pub struct DDLOutputManager {
    /// Configuration for output generation
    config: OutputConfig,
}

/// Configuration options for DDL output
#[derive(Debug, Clone)]
pub struct OutputConfig {
    /// Directory path for output files
    pub output_directory: String,
    /// Whether to include detailed comments
    pub include_detailed_comments: bool,
    /// Whether to generate separate files for different components
    pub separate_files: bool,
    /// Whether to include transformation report
    pub include_transformation_report: bool,
    /// Whether to include warnings and recommendations
    pub include_warnings: bool,
    /// File format for output (SQL, Markdown, etc.)
    pub output_format: OutputFormat,
}

/// Output format options
#[derive(Debug, Clone)]
pub enum OutputFormat {
    /// Pure SQL DDL files
    SQL,
    /// Markdown documentation with embedded SQL
    Markdown,
    /// Combined format with both SQL and documentation
    Combined,
}

/// Complete DDL output package
#[derive(Debug, Clone)]
pub struct DDLOutputPackage {
    /// Generated DDL statements
    pub ddl: GeneratedDDL,
    /// Constraint analysis results
    pub constraints: ConstraintAnalysisResult,
    /// Index analysis results
    pub indexes: IndexAnalysisResult,
    /// Transformation report
    pub transformation_report: TransformationReport,
    /// Output file paths
    pub file_paths: Vec<String>,
}

/// Detailed transformation report showing original vs normalized structures
#[derive(Debug, Clone)]
pub struct TransformationReport {
    /// Original Firestore collections analyzed
    pub original_collections: Vec<CollectionSummary>,
    /// Generated normalized tables
    pub normalized_tables: Vec<TableSummary>,
    /// Applied transformations
    pub transformations: Vec<TransformationSummary>,
    /// Type conflicts and resolutions
    pub type_conflicts: Vec<TypeConflictSummary>,
    /// Normalization opportunities applied
    pub normalization_applied: Vec<NormalizationSummary>,
    /// Overall statistics
    pub statistics: TransformationStatistics,
}

/// Summary of an original Firestore collection
#[derive(Debug, Clone)]
pub struct CollectionSummary {
    /// Collection name
    pub name: String,
    /// Number of documents
    pub document_count: u64,
    /// Unique field names
    pub field_count: usize,
    /// Average document size
    pub avg_size_bytes: f64,
    /// Nested subcollections
    pub subcollections: Vec<String>,
}

/// Summary of a generated table
#[derive(Debug, Clone)]
pub struct TableSummary {
    /// Table name
    pub name: String,
    /// Number of columns
    pub column_count: usize,
    /// Primary key columns
    pub primary_key: Vec<String>,
    /// Foreign key relationships
    pub foreign_key_count: usize,
    /// Index count
    pub index_count: usize,
    /// Source collection(s)
    pub source_collections: Vec<String>,
}

/// Summary of a transformation applied
#[derive(Debug, Clone)]
pub struct TransformationSummary {
    /// Type of transformation
    pub transformation_type: String,
    /// Source field or collection
    pub source: String,
    /// Target table and column
    pub target: String,
    /// Description of the transformation
    pub description: String,
    /// Reason for the transformation
    pub reason: String,
}

/// Summary of a type conflict and its resolution
#[derive(Debug, Clone)]
pub struct TypeConflictSummary {
    /// Field path where conflict occurred
    pub field_path: String,
    /// Conflicting types found
    pub conflicting_types: Vec<String>,
    /// Chosen resolution
    pub resolution: String,
    /// Confidence in resolution
    pub confidence: f64,
}

/// Summary of normalization applied
#[derive(Debug, Clone)]
pub struct NormalizationSummary {
    /// Original collection
    pub collection: String,
    /// Field that was normalized
    pub field: String,
    /// Normalization type applied
    pub normalization_type: String,
    /// New table created
    pub new_table: String,
    /// Impact assessment
    pub impact: String,
}

/// Overall transformation statistics
#[derive(Debug, Clone)]
pub struct TransformationStatistics {
    /// Total collections processed
    pub collections_processed: u32,
    /// Total tables generated
    pub tables_generated: u32,
    /// Total fields transformed
    pub fields_transformed: u32,
    /// Total relationships created
    pub relationships_created: u32,
    /// Total constraints generated
    pub constraints_generated: u32,
    /// Total indexes recommended
    pub indexes_recommended: u32,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            output_directory: "./ddl_output".to_string(),
            include_detailed_comments: true,
            separate_files: true,
            include_transformation_report: true,
            include_warnings: true,
            output_format: OutputFormat::Combined,
        }
    }
}

impl DDLOutputManager {
    /// Create a new DDL output manager with default configuration
    pub fn new() -> Self {
        Self {
            config: OutputConfig::default(),
        }
    }

    /// Create a new DDL output manager with custom configuration
    pub fn with_config(config: OutputConfig) -> Self {
        Self { config }
    }

    /// Generate complete DDL output package
    pub fn generate_output_package(
        &self,
        schema: &NormalizedSchema,
        analysis: &SchemaAnalysis,
    ) -> FireupResult<DDLOutputPackage> {
        // Generate DDL statements
        let ddl_generator = DDLGenerator::new();
        let ddl = ddl_generator.generate_ddl(schema)?;

        // Generate constraint analysis
        let constraint_generator = ConstraintGenerator::new();
        let constraints = constraint_generator.generate_constraints(schema, analysis)?;

        // Generate index analysis
        let index_generator = IndexGenerator::new();
        let indexes = index_generator.generate_indexes(schema, analysis)?;

        // Generate transformation report
        let transformation_report = self.generate_transformation_report(schema, analysis)?;

        // Create output directory
        fs::create_dir_all(&self.config.output_directory)?;

        // Generate output files
        let file_paths = self.write_output_files(&ddl, &constraints, &indexes, &transformation_report)?;

        Ok(DDLOutputPackage {
            ddl,
            constraints,
            indexes,
            transformation_report,
            file_paths,
        })
    }

    /// Generate transformation report
    fn generate_transformation_report(
        &self,
        schema: &NormalizedSchema,
        analysis: &SchemaAnalysis,
    ) -> FireupResult<TransformationReport> {
        // Generate collection summaries
        let original_collections: Vec<CollectionSummary> = analysis.collections
            .iter()
            .map(|col| CollectionSummary {
                name: col.name.clone(),
                document_count: col.document_count,
                field_count: col.field_names.len(),
                avg_size_bytes: col.avg_document_size,
                subcollections: col.subcollections.clone(),
            })
            .collect();

        // Generate table summaries
        let normalized_tables: Vec<TableSummary> = schema.tables
            .iter()
            .map(|table| TableSummary {
                name: table.name.clone(),
                column_count: table.columns.len(),
                primary_key: table.primary_key.clone(),
                foreign_key_count: table.foreign_keys.len(),
                index_count: table.indexes.len(),
                source_collections: vec![table.name.clone()], // Simplified mapping
            })
            .collect();

        // Generate transformation summaries
        let transformations = self.generate_transformation_summaries(schema, analysis)?;

        // Generate type conflict summaries
        let type_conflicts: Vec<TypeConflictSummary> = analysis.field_types
            .iter()
            .filter(|ft| ft.type_frequencies.len() > 1)
            .map(|ft| TypeConflictSummary {
                field_path: ft.field_path.clone(),
                conflicting_types: ft.type_frequencies.keys().cloned().collect(),
                resolution: ft.recommended_type.to_sql(),
                confidence: 0.8, // Simplified confidence calculation
            })
            .collect();

        // Generate normalization summaries
        let normalization_applied: Vec<NormalizationSummary> = analysis.normalization_opportunities
            .iter()
            .map(|norm| NormalizationSummary {
                collection: norm.collection.clone(),
                field: norm.field_path.clone(),
                normalization_type: format!("{:?}", norm.normalization_type),
                new_table: format!("{}_normalized", norm.collection),
                impact: format!("{:?}", norm.impact),
            })
            .collect();

        // Calculate statistics
        let statistics = TransformationStatistics {
            collections_processed: analysis.metadata.total_collections,
            tables_generated: schema.metadata.table_count,
            fields_transformed: analysis.field_types.len() as u32,
            relationships_created: schema.metadata.relationship_count,
            constraints_generated: schema.constraints.len() as u32,
            indexes_recommended: schema.tables.iter().map(|t| t.indexes.len()).sum::<usize>() as u32,
        };

        Ok(TransformationReport {
            original_collections,
            normalized_tables,
            transformations,
            type_conflicts,
            normalization_applied,
            statistics,
        })
    }

    /// Generate transformation summaries
    fn generate_transformation_summaries(
        &self,
        schema: &NormalizedSchema,
        analysis: &SchemaAnalysis,
    ) -> FireupResult<Vec<TransformationSummary>> {
        let mut transformations = Vec::new();

        // Document field type transformations
        for field_type in &analysis.field_types {
            if field_type.type_frequencies.len() > 1 {
                transformations.push(TransformationSummary {
                    transformation_type: "Type Resolution".to_string(),
                    source: field_type.field_path.clone(),
                    target: format!("PostgreSQL {}", field_type.recommended_type.to_sql()),
                    description: format!(
                        "Resolved type conflict by choosing {} based on frequency analysis",
                        field_type.recommended_type.to_sql()
                    ),
                    reason: "Multiple data types detected for the same field".to_string(),
                });
            }
        }

        // Document normalization transformations
        for opportunity in &analysis.normalization_opportunities {
            transformations.push(TransformationSummary {
                transformation_type: format!("{:?}", opportunity.normalization_type),
                source: format!("{}.{}", opportunity.collection, opportunity.field_path),
                target: format!("{}_normalized table", opportunity.collection),
                description: opportunity.description.clone(),
                reason: "Database normalization to reduce redundancy".to_string(),
            });
        }

        // Document relationship transformations
        for relationship in &analysis.relationships {
            transformations.push(TransformationSummary {
                transformation_type: "Relationship Creation".to_string(),
                source: format!("{}.{}", relationship.from_collection, relationship.reference_field),
                target: format!("Foreign key to {}", relationship.to_collection),
                description: format!(
                    "Created {:?} relationship between {} and {}",
                    relationship.relationship_type,
                    relationship.from_collection,
                    relationship.to_collection
                ),
                reason: format!("Detected relationship with {:.1}% confidence", relationship.confidence * 100.0),
            });
        }

        Ok(transformations)
    }

    /// Write output files to disk
    fn write_output_files(
        &self,
        ddl: &GeneratedDDL,
        constraints: &ConstraintAnalysisResult,
        indexes: &IndexAnalysisResult,
        report: &TransformationReport,
    ) -> FireupResult<Vec<String>> {
        let mut file_paths = Vec::new();

        match self.config.output_format {
            OutputFormat::SQL => {
                file_paths.extend(self.write_sql_files(ddl, constraints, indexes)?);
            }
            OutputFormat::Markdown => {
                file_paths.extend(self.write_markdown_files(ddl, constraints, indexes, report)?);
            }
            OutputFormat::Combined => {
                file_paths.extend(self.write_sql_files(ddl, constraints, indexes)?);
                file_paths.extend(self.write_markdown_files(ddl, constraints, indexes, report)?);
            }
        }

        Ok(file_paths)
    }

    /// Write SQL DDL files
    fn write_sql_files(
        &self,
        ddl: &GeneratedDDL,
        constraints: &ConstraintAnalysisResult,
        indexes: &IndexAnalysisResult,
    ) -> FireupResult<Vec<String>> {
        let mut file_paths = Vec::new();

        if self.config.separate_files {
            // Write separate files for each component
            let tables_path = format!("{}/01_tables.sql", self.config.output_directory);
            fs::write(&tables_path, ddl.table_statements.join("\n\n"))?;
            file_paths.push(tables_path);

            let fk_path = format!("{}/02_foreign_keys.sql", self.config.output_directory);
            fs::write(&fk_path, ddl.foreign_key_statements.join("\n"))?;
            file_paths.push(fk_path);

            let constraints_path = format!("{}/03_constraints.sql", self.config.output_directory);
            let constraint_sql: Vec<String> = constraints.constraints
                .iter()
                .map(|c| format!("-- {}", c.name))
                .collect();
            fs::write(&constraints_path, constraint_sql.join("\n"))?;
            file_paths.push(constraints_path);

            let indexes_path = format!("{}/04_indexes.sql", self.config.output_directory);
            let index_sql: Vec<String> = indexes.indexes
                .iter()
                .map(|idx| format!("CREATE INDEX {} ON table_name ({});", idx.name, idx.columns.join(", ")))
                .collect();
            fs::write(&indexes_path, index_sql.join("\n"))?;
            file_paths.push(indexes_path);
        } else {
            // Write single combined SQL file
            let combined_path = format!("{}/schema.sql", self.config.output_directory);
            fs::write(&combined_path, ddl.to_string())?;
            file_paths.push(combined_path);
        }

        Ok(file_paths)
    }

    /// Write Markdown documentation files
    fn write_markdown_files(
        &self,
        ddl: &GeneratedDDL,
        constraints: &ConstraintAnalysisResult,
        indexes: &IndexAnalysisResult,
        report: &TransformationReport,
    ) -> FireupResult<Vec<String>> {
        let mut file_paths = Vec::new();

        // Write main documentation file
        let doc_path = format!("{}/README.md", self.config.output_directory);
        let documentation = self.generate_documentation(ddl, constraints, indexes, report)?;
        fs::write(&doc_path, documentation)?;
        file_paths.push(doc_path);

        // Write transformation report
        if self.config.include_transformation_report {
            let report_path = format!("{}/transformation_report.md", self.config.output_directory);
            let report_content = self.generate_transformation_report_markdown(report)?;
            fs::write(&report_path, report_content)?;
            file_paths.push(report_path);
        }

        // Write warnings and recommendations
        if self.config.include_warnings {
            let warnings_path = format!("{}/warnings_and_recommendations.md", self.config.output_directory);
            let warnings_content = self.generate_warnings_markdown(ddl, constraints, indexes)?;
            fs::write(&warnings_path, warnings_content)?;
            file_paths.push(warnings_path);
        }

        Ok(file_paths)
    }

    /// Generate main documentation content
    fn generate_documentation(
        &self,
        ddl: &GeneratedDDL,
        constraints: &ConstraintAnalysisResult,
        indexes: &IndexAnalysisResult,
        report: &TransformationReport,
    ) -> FireupResult<String> {
        let mut doc = String::new();

        doc.push_str("# PostgreSQL Schema Documentation\n\n");
        doc.push_str(&format!("Generated on: {}\n\n", Utc::now().format("%Y-%m-%d %H:%M:%S UTC")));

        // Overview section
        doc.push_str("## Overview\n\n");
        doc.push_str("This schema was generated from Firestore backup data analysis.\n\n");
        
        let summary = ddl.summary();
        doc.push_str(&format!("- **Tables**: {}\n", summary.table_count));
        doc.push_str(&format!("- **Foreign Keys**: {}\n", summary.foreign_key_count));
        doc.push_str(&format!("- **Indexes**: {}\n", summary.index_count));
        doc.push_str(&format!("- **Constraints**: {}\n", summary.constraint_count));
        doc.push_str(&format!("- **Warnings**: {}\n\n", summary.warning_count));

        // Transformation statistics
        doc.push_str("## Transformation Statistics\n\n");
        doc.push_str(&format!("- **Collections Processed**: {}\n", report.statistics.collections_processed));
        doc.push_str(&format!("- **Tables Generated**: {}\n", report.statistics.tables_generated));
        doc.push_str(&format!("- **Fields Transformed**: {}\n", report.statistics.fields_transformed));
        doc.push_str(&format!("- **Relationships Created**: {}\n\n", report.statistics.relationships_created));

        // Tables section
        doc.push_str("## Tables\n\n");
        for table_summary in &report.normalized_tables {
            doc.push_str(&format!("### {}\n\n", table_summary.name));
            doc.push_str(&format!("- **Columns**: {}\n", table_summary.column_count));
            doc.push_str(&format!("- **Primary Key**: {}\n", table_summary.primary_key.join(", ")));
            doc.push_str(&format!("- **Foreign Keys**: {}\n", table_summary.foreign_key_count));
            doc.push_str(&format!("- **Indexes**: {}\n\n", table_summary.index_count));
        }

        // Usage instructions
        doc.push_str("## Usage Instructions\n\n");
        doc.push_str("1. Review the generated DDL files in this directory\n");
        doc.push_str("2. Check the warnings and recommendations file\n");
        doc.push_str("3. Execute the SQL files in the following order:\n");
        doc.push_str("   - `01_tables.sql` - Create tables\n");
        doc.push_str("   - `02_foreign_keys.sql` - Add foreign key constraints\n");
        doc.push_str("   - `03_constraints.sql` - Add additional constraints\n");
        doc.push_str("   - `04_indexes.sql` - Create indexes\n\n");

        Ok(doc)
    }

    /// Generate transformation report markdown
    fn generate_transformation_report_markdown(&self, report: &TransformationReport) -> FireupResult<String> {
        let mut content = String::new();

        content.push_str("# Transformation Report\n\n");
        content.push_str("This report details the transformations applied during the migration from Firestore to PostgreSQL.\n\n");

        // Original collections
        content.push_str("## Original Firestore Collections\n\n");
        for collection in &report.original_collections {
            content.push_str(&format!("### {}\n\n", collection.name));
            content.push_str(&format!("- **Documents**: {}\n", collection.document_count));
            content.push_str(&format!("- **Fields**: {}\n", collection.field_count));
            content.push_str(&format!("- **Avg Size**: {:.2} bytes\n", collection.avg_size_bytes));
            if !collection.subcollections.is_empty() {
                content.push_str(&format!("- **Subcollections**: {}\n", collection.subcollections.join(", ")));
            }
            content.push_str("\n");
        }

        // Transformations applied
        content.push_str("## Transformations Applied\n\n");
        for transformation in &report.transformations {
            content.push_str(&format!("### {} - {}\n\n", transformation.transformation_type, transformation.source));
            content.push_str(&format!("**Target**: {}\n\n", transformation.target));
            content.push_str(&format!("**Description**: {}\n\n", transformation.description));
            content.push_str(&format!("**Reason**: {}\n\n", transformation.reason));
        }

        // Type conflicts
        if !report.type_conflicts.is_empty() {
            content.push_str("## Type Conflicts Resolved\n\n");
            for conflict in &report.type_conflicts {
                content.push_str(&format!("### {}\n\n", conflict.field_path));
                content.push_str(&format!("**Conflicting Types**: {}\n\n", conflict.conflicting_types.join(", ")));
                content.push_str(&format!("**Resolution**: {}\n\n", conflict.resolution));
                content.push_str(&format!("**Confidence**: {:.1}%\n\n", conflict.confidence * 100.0));
            }
        }

        Ok(content)
    }

    /// Generate warnings and recommendations markdown
    fn generate_warnings_markdown(
        &self,
        ddl: &GeneratedDDL,
        constraints: &ConstraintAnalysisResult,
        indexes: &IndexAnalysisResult,
    ) -> FireupResult<String> {
        let mut content = String::new();

        content.push_str("# Warnings and Recommendations\n\n");

        // Schema warnings
        if !ddl.warnings.is_empty() {
            content.push_str("## Schema Warnings\n\n");
            for warning in &ddl.warnings {
                content.push_str(&format!("### {} - {}\n\n", 
                    match warning.level {
                        WarningLevel::Error => "❌ ERROR",
                        WarningLevel::Warning => "⚠️ WARNING", 
                        WarningLevel::Info => "ℹ️ INFO",
                    },
                    warning.context
                ));
                content.push_str(&format!("{}\n\n", warning.message));
                if let Some(suggestion) = &warning.suggestion {
                    content.push_str(&format!("**Suggestion**: {}\n\n", suggestion));
                }
            }
        }

        // Constraint recommendations
        if !constraints.recommendations.is_empty() {
            content.push_str("## Constraint Recommendations\n\n");
            for rec in &constraints.recommendations {
                content.push_str(&format!("### {} - {}\n\n", 
                    format!("{:?}", rec.constraint_type),
                    rec.table
                ));
                content.push_str(&format!("**Columns**: {}\n\n", rec.columns.join(", ")));
                content.push_str(&format!("**Reason**: {}\n\n", rec.reason));
                content.push_str(&format!("**Confidence**: {:.1}%\n\n", rec.confidence * 100.0));
                content.push_str(&format!("**Suggested SQL**:\n```sql\n{}\n```\n\n", rec.suggested_definition));
            }
        }

        // Index recommendations
        if !indexes.recommendations.is_empty() {
            content.push_str("## Index Recommendations\n\n");
            for rec in &indexes.recommendations {
                content.push_str(&format!("### {}\n\n", rec.index.name));
                content.push_str(&format!("**Columns**: {}\n\n", rec.index.columns.join(", ")));
                content.push_str(&format!("**Reason**: {}\n\n", rec.reason));
                content.push_str(&format!("**Confidence**: {:.1}%\n\n", rec.confidence * 100.0));
                content.push_str(&format!("**Performance Impact**: {:?}\n\n", rec.performance_impact));
                content.push_str(&format!("**Storage Overhead**: {:?}\n\n", rec.storage_overhead));
            }
        }

        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use chrono::Utc;
    use uuid::Uuid;
    use tempfile::TempDir;
    use std::path::Path;

    fn create_test_schema() -> NormalizedSchema {
        let mut table = TableDefinition::new("users".to_string());
        table.add_column(ColumnDefinition::new("id".to_string(), PostgreSQLType::Uuid));
        table.add_column(ColumnDefinition::new("email".to_string(), PostgreSQLType::Varchar(Some(255))));
        
        NormalizedSchema {
            tables: vec![table],
            relationships: Vec::new(),
            constraints: Vec::new(),
            warnings: vec![
                SchemaWarning {
                    level: WarningLevel::Warning,
                    message: "Test warning".to_string(),
                    context: "users.email".to_string(),
                    suggestion: Some("Consider adding unique constraint".to_string()),
                }
            ],
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
        
        analysis.add_collection(CollectionAnalysis {
            name: "users".to_string(),
            document_count: 100,
            field_names: vec!["id".to_string(), "email".to_string()],
            avg_document_size: 256.0,
            subcollections: Vec::new(),
        });
        
        analysis
    }

    #[test]
    fn test_generate_transformation_report() {
        let temp_dir = TempDir::new().unwrap();
        let config = OutputConfig {
            output_directory: temp_dir.path().to_string_lossy().to_string(),
            ..Default::default()
        };
        
        let manager = DDLOutputManager::with_config(config);
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let report = manager.generate_transformation_report(&schema, &analysis).unwrap();
        
        assert_eq!(report.original_collections.len(), 1);
        assert_eq!(report.normalized_tables.len(), 1);
        assert_eq!(report.statistics.collections_processed, 1);
        assert_eq!(report.statistics.tables_generated, 1);
    }

    #[test]
    fn test_generate_output_package() {
        let temp_dir = TempDir::new().unwrap();
        let config = OutputConfig {
            output_directory: temp_dir.path().to_string_lossy().to_string(),
            separate_files: true,
            include_transformation_report: true,
            ..Default::default()
        };
        
        let manager = DDLOutputManager::with_config(config);
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let package = manager.generate_output_package(&schema, &analysis).unwrap();
        
        assert!(!package.file_paths.is_empty());
        assert_eq!(package.ddl.table_statements.len(), 1);
        
        // Check that files were created
        for file_path in &package.file_paths {
            assert!(Path::new(file_path).exists());
        }
    }

    #[test]
    fn test_markdown_generation() {
        let temp_dir = TempDir::new().unwrap();
        let config = OutputConfig {
            output_directory: temp_dir.path().to_string_lossy().to_string(),
            output_format: OutputFormat::Markdown,
            ..Default::default()
        };
        
        let manager = DDLOutputManager::with_config(config);
        let schema = create_test_schema();
        let analysis = create_test_analysis();
        
        let package = manager.generate_output_package(&schema, &analysis).unwrap();
        
        // Should generate markdown files
        let readme_exists = package.file_paths.iter().any(|p| p.ends_with("README.md"));
        assert!(readme_exists);
        
        let warnings_exists = package.file_paths.iter().any(|p| p.ends_with("warnings_and_recommendations.md"));
        assert!(warnings_exists);
    }
}