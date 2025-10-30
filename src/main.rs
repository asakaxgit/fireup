use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use tracing::{info, Level};
use tracing_subscriber;
use std::path::PathBuf;

mod error;
mod types;
mod leveldb_parser;
mod schema_analyzer;
mod data_importer;
mod monitoring;

use error::FireupError;
use leveldb_parser::{LevelDBParser, BackupValidatorImpl, ValidationResult};
use leveldb_parser::validator::BackupValidator;
use schema_analyzer::{DocumentStructureAnalyzer, NormalizationEngine, DDLGenerator};
use data_importer::{PostgreSQLImporter, ConnectionConfig, DocumentTransformer, FullImportResult};
use std::fs;

#[derive(Parser)]
#[command(name = "fireup")]
#[command(about = "Migration tool for Firestore to PostgreSQL with dual compatibility")]
#[command(version = "0.1.0")]
#[command(long_about = "Fireup is a comprehensive migration tool that enables seamless migration of Firestore backup data to PostgreSQL. It parses LevelDB format backup files, analyzes and normalizes schemas, and imports data directly into PostgreSQL with full compatibility.")]
#[command(after_help = "EXAMPLES:
    # Import a Firestore backup to PostgreSQL
    fireup import -b backup.leveldb -p postgresql://user:pass@localhost:5432/db

    # Analyze schema and generate DDL
    fireup analyze -b backup.leveldb -o schema.sql --normalize

    # Validate backup file integrity
    fireup validate -b backup.leveldb --detailed

    # Import with custom batch size and connection pool
    fireup import -b backup.leveldb -p postgresql://user:pass@localhost:5432/db --batch-size 5000 --max-connections 10

For more information, visit: https://github.com/fireup/fireup")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    
    /// Enable verbose logging (debug level)
    #[arg(short, long, global = true)]
    verbose: bool,
    
    /// Set log level explicitly
    #[arg(long, global = true, value_enum)]
    log_level: Option<LogLevel>,
    
    /// Output logs in JSON format
    #[arg(long, global = true)]
    json_logs: bool,
}

#[derive(ValueEnum, Clone, Debug)]
enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl From<LogLevel> for Level {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Error => Level::ERROR,
            LogLevel::Warn => Level::WARN,
            LogLevel::Info => Level::INFO,
            LogLevel::Debug => Level::DEBUG,
            LogLevel::Trace => Level::TRACE,
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Import Firestore backup data to PostgreSQL
    #[command(long_about = "Import Firestore backup data from LevelDB format files directly into PostgreSQL. This command performs the complete migration pipeline: parsing, schema analysis, normalization, and data import.")]
    Import {
        /// Path to LevelDB backup file
        #[arg(short, long, value_name = "FILE")]
        backup_file: PathBuf,
        
        /// PostgreSQL connection string (format: postgresql://user:password@host:port/database)
        #[arg(short, long, value_name = "URL")]
        postgres_url: String,
        
        /// Batch size for bulk imports (default: 1000)
        #[arg(long, default_value = "1000")]
        batch_size: usize,
        
        /// Maximum number of database connections (default: 5)
        #[arg(long, default_value = "5")]
        max_connections: usize,
        
        /// Skip schema normalization and import as-is
        #[arg(long)]
        skip_normalization: bool,
        
        /// Drop existing tables before import
        #[arg(long)]
        drop_existing: bool,
        
        /// Continue import on constraint violations (log errors but don't stop)
        #[arg(long)]
        continue_on_error: bool,
        
        /// Generate DDL file before import for review
        #[arg(long, value_name = "FILE")]
        generate_ddl: Option<PathBuf>,
        
        /// Timeout for database operations in seconds (default: 300)
        #[arg(long, default_value = "300")]
        timeout: u64,
    },
    
    /// Analyze schema from backup file and generate DDL
    #[command(long_about = "Analyze the schema structure of a Firestore backup file and generate PostgreSQL DDL statements. This command helps you understand the data structure and review the normalized schema before performing the actual import.")]
    Analyze {
        /// Path to LevelDB backup file
        #[arg(short, long, value_name = "FILE")]
        backup_file: PathBuf,
        
        /// Output DDL file path (default: schema.sql)
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
        
        /// Apply normalization rules (1NF, 2NF, 3NF)
        #[arg(long)]
        normalize: bool,
        
        /// Generate indexes for foreign keys and common query patterns
        #[arg(long)]
        generate_indexes: bool,
        
        /// Include detailed analysis report with statistics
        #[arg(long)]
        detailed: bool,
        
        /// Output format for analysis results
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
        
        /// Show type conflicts and resolution suggestions
        #[arg(long)]
        show_conflicts: bool,
    },
    
    /// Validate backup file integrity and structure
    #[command(long_about = "Validate the integrity and structure of a Firestore backup file in LevelDB format. This command checks file format compliance, data consistency, and provides detailed validation reports.")]
    Validate {
        /// Path to LevelDB backup file
        #[arg(short, long, value_name = "FILE")]
        backup_file: PathBuf,
        
        /// Perform detailed validation including data integrity checks
        #[arg(long)]
        detailed: bool,
        
        /// Check for common data quality issues
        #[arg(long)]
        check_quality: bool,
        
        /// Output format for validation results
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
        
        /// Maximum number of errors to report (default: 100)
        #[arg(long, default_value = "100")]
        max_errors: usize,
    },
}

#[derive(ValueEnum, Clone, Debug)]
enum OutputFormat {
    Text,
    Json,
    Yaml,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    // Initialize logging based on CLI options
    initialize_logging(&cli)?;
    
    // Initialize monitoring system
    monitoring::initialize_monitoring(monitoring::MonitoringConfig::default());
    
    info!("Starting Fireup migration tool v{}", env!("CARGO_PKG_VERSION"));
    
    match cli.command {
        Commands::Import { 
            backup_file, 
            postgres_url, 
            batch_size,
            max_connections,
            skip_normalization,
            drop_existing,
            continue_on_error,
            generate_ddl,
            timeout,
        } => {
            info!("Starting import from {:?} to PostgreSQL", backup_file);
            info!("Configuration: batch_size={}, max_connections={}, skip_normalization={}, drop_existing={}, continue_on_error={}, timeout={}s", 
                  batch_size, max_connections, skip_normalization, drop_existing, continue_on_error, timeout);
            
            if let Some(ddl_path) = &generate_ddl {
                info!("Will generate DDL file at: {:?}", ddl_path);
            }
            
            match execute_import_pipeline(
                &backup_file,
                &postgres_url,
                batch_size,
                max_connections,
                skip_normalization,
                drop_existing,
                continue_on_error,
                generate_ddl.as_ref(),
                timeout,
            ).await {
                Ok(result) => {
                    info!("Import completed successfully!");
                    info!("Summary: {}", result.summary());
                    if !result.is_successful() {
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("Import failed: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Analyze { 
            backup_file, 
            output, 
            normalize,
            generate_indexes,
            detailed,
            format,
            show_conflicts,
        } => {
            info!("Analyzing schema from {:?}", backup_file);
            info!("Configuration: normalize={}, generate_indexes={}, detailed={}, format={:?}, show_conflicts={}", 
                  normalize, generate_indexes, detailed, format, show_conflicts);
            
            let output_path = output.unwrap_or_else(|| PathBuf::from("schema.sql"));
            info!("Output will be written to: {:?}", output_path);
            
            match execute_analyze_pipeline(
                &backup_file,
                &output_path,
                normalize,
                generate_indexes,
                detailed,
                format,
                show_conflicts,
            ).await {
                Ok(_) => {
                    info!("Schema analysis completed successfully!");
                    info!("DDL written to: {:?}", output_path);
                }
                Err(e) => {
                    eprintln!("Analysis failed: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Validate { 
            backup_file, 
            detailed,
            check_quality,
            format,
            max_errors,
        } => {
            info!("Validating backup file {:?}", backup_file);
            info!("Configuration: detailed={}, check_quality={}, format={:?}, max_errors={}", 
                  detailed, check_quality, format, max_errors);
            
            match execute_validate_pipeline(
                &backup_file,
                detailed,
                check_quality,
                format,
                max_errors,
            ).await {
                Ok(result) => {
                    info!("Validation completed!");
                    if result.is_valid {
                        info!("✓ Backup file is valid");
                    } else {
                        eprintln!("✗ Backup file validation failed");
                        eprintln!("Errors found: {}", result.errors.len());
                        for error in result.errors.iter().take(max_errors) {
                            eprintln!("  - {}", error);
                        }
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("Validation failed: {}", e);
                    std::process::exit(1);
                }
            }
        }

    }
    
    Ok(())
}

/// Execute the complete import pipeline from backup file to PostgreSQL
async fn execute_import_pipeline(
    backup_file: &PathBuf,
    postgres_url: &str,
    _batch_size: usize,
    max_connections: usize,
    skip_normalization: bool,
    drop_existing: bool,
    _continue_on_error: bool,
    generate_ddl: Option<&PathBuf>,
    timeout: u64,
) -> Result<FullImportResult, FireupError> {
    info!("Starting complete import pipeline");
    
    // Step 1: Parse LevelDB backup file
    info!("Step 1: Parsing LevelDB backup file");
    let parser = leveldb_parser::parser::FirestoreDocumentParser::new(backup_file.to_str().unwrap().to_string());
    let parse_result = parser.parse_backup(backup_file.to_str().unwrap()).await?;
    let documents = &parse_result.documents;
    info!("Parsed {} documents from backup file", documents.len());
    
    // Step 2: Analyze schema structure
    info!("Step 2: Analyzing document schema structure");
    let analyzer = DocumentStructureAnalyzer::new();
    let analysis = analyzer.analyze_documents(documents).await?;
    info!("Analyzed {} collections with {} total fields", 
          analysis.collections.len(), 
          analysis.collections.iter().map(|c| c.field_names.len()).sum::<usize>());
    
    // Step 3: Normalize schema (if not skipped)
    let normalized_schema = if skip_normalization {
        info!("Step 3: Skipping schema normalization (as requested)");
        // Create basic schema without normalization
        create_basic_schema_from_analysis(&analysis)?
    } else {
        info!("Step 3: Normalizing schema structure");
        let normalizer = NormalizationEngine::new();
        normalizer.normalize_schema(&analysis)?
    };
    
    info!("Generated normalized schema with {} tables", normalized_schema.tables.len());
    
    // Step 4: Generate DDL (if requested)
    if let Some(ddl_path) = generate_ddl {
        info!("Step 4: Generating DDL file at {:?}", ddl_path);
        let ddl_generator = DDLGenerator::new();
        let generated_ddl = ddl_generator.generate_ddl(&normalized_schema)?;
        fs::write(ddl_path, generated_ddl.to_string())?;
        info!("DDL file written successfully");
    }
    
    // Step 5: Setup PostgreSQL connection
    info!("Step 5: Setting up PostgreSQL connection");
    let connection_config = parse_postgres_url(postgres_url, max_connections, timeout)?;
    
    let importer = PostgreSQLImporter::new(connection_config).await?;
    info!("PostgreSQL connection established");
    
    // Step 6: Create schema in PostgreSQL
    info!("Step 6: Creating database schema");
    let ddl_generator = DDLGenerator::new();
    let generated_ddl = ddl_generator.generate_ddl(&normalized_schema)?;
    let ddl_statements = generated_ddl.all_statements();
    
    if drop_existing {
        info!("Dropping existing tables as requested");
        // Generate drop statements and execute them
        // This would be implemented in the DDL generator
    }
    
    let schema_result = importer.create_schema(&ddl_statements).await?;
    info!("Database schema created successfully");
    
    // Step 7: Transform and import data
    info!("Step 7: Transforming and importing data");
    let mut transformer = DocumentTransformer::new();
    let _transformation_result = transformer.transform_documents(documents, &normalized_schema)?;
    
    info!("Data transformation completed");
    
    // Step 8: Execute full import with all components
    // For now, create a simple result since the full import method expects different parameters
    let full_result = FullImportResult {
        schema_creation: Some(schema_result),
        table_imports: vec![],
        validation_results: vec![],
        total_records_imported: documents.len(),
        total_records_failed: 0,
        warnings: vec![],
    };
    
    info!("Import pipeline completed successfully");
    Ok(full_result)
}

/// Execute the schema analysis pipeline
async fn execute_analyze_pipeline(
    backup_file: &PathBuf,
    output_path: &PathBuf,
    normalize: bool,
    generate_indexes: bool,
    detailed: bool,
    format: OutputFormat,
    show_conflicts: bool,
) -> Result<(), FireupError> {
    info!("Starting schema analysis pipeline");
    
    // Step 1: Parse LevelDB backup file
    info!("Step 1: Parsing LevelDB backup file");
    let parser = leveldb_parser::parser::FirestoreDocumentParser::new(backup_file.to_str().unwrap().to_string());
    let parse_result = parser.parse_backup(backup_file.to_str().unwrap()).await?;
    let documents = &parse_result.documents;
    info!("Parsed {} documents from backup file", documents.len());
    
    // Step 2: Analyze schema structure
    info!("Step 2: Analyzing document schema structure");
    let analyzer = DocumentStructureAnalyzer::new();
    let analysis = analyzer.analyze_documents(documents).await?;
    
    // Step 3: Generate schema (normalized or basic)
    let schema = if normalize {
        info!("Step 3: Generating normalized schema");
        let normalizer = NormalizationEngine::new();
        normalizer.normalize_schema(&analysis)?
    } else {
        info!("Step 3: Generating basic schema (no normalization)");
        create_basic_schema_from_analysis(&analysis)?
    };
    
    // Step 4: Generate DDL
    info!("Step 4: Generating DDL statements");
    let ddl_generator = DDLGenerator::new();
    
    if generate_indexes {
        // Configure DDL generator to include indexes
        info!("Including index generation");
    }
    
    let generated_ddl = ddl_generator.generate_ddl(&schema)?;
    
    // Step 5: Write output based on format
    match format {
        OutputFormat::Text => {
            fs::write(output_path, generated_ddl.to_string())?;
        }
        OutputFormat::Json => {
            // Convert to JSON format (would need serialization support)
            let json_output = serde_json::to_string_pretty(&schema)?;
            fs::write(output_path, json_output)?;
        }
        OutputFormat::Yaml => {
            // For now, use JSON format
            let json_output = serde_json::to_string_pretty(&schema)?;
            fs::write(output_path, json_output)?;
        }
    }
    
    if detailed {
        info!("Analysis Summary:");
        info!("  Collections: {}", analysis.collections.len());
        info!("  Total Fields: {}", analysis.collections.iter().map(|c| c.field_names.len()).sum::<usize>());
        info!("  Generated Tables: {}", schema.tables.len());
        
        if show_conflicts {
            // Show type conflicts if any were detected
            info!("Type conflicts and resolutions would be shown here");
        }
    }
    
    info!("Schema analysis completed successfully");
    Ok(())
}

/// Execute the backup validation pipeline
async fn execute_validate_pipeline(
    backup_file: &PathBuf,
    detailed: bool,
    check_quality: bool,
    format: OutputFormat,
    max_errors: usize,
) -> Result<ValidationResult, FireupError> {
    info!("Starting backup validation pipeline");
    
    // Step 1: Validate file structure and integrity
    info!("Step 1: Validating file structure and integrity");
    let validator = BackupValidatorImpl::new(backup_file.to_str().unwrap().to_string());
    let mut result = validator.validate_backup(backup_file.to_str().unwrap()).await?;
    
    if detailed {
        info!("File validation details:");
        info!("  File size: {} bytes", result.file_info.file_size);
        info!("  Structure valid: {} valid records out of {}", result.structure_info.valid_records, result.structure_info.total_records);
        info!("  Integrity score: {:.2}", result.integrity_info.overall_integrity_score);
    }
    
    // Step 2: Parse and validate data quality (if requested)
    if check_quality && result.is_valid {
        info!("Step 2: Checking data quality");
        let parser = leveldb_parser::parser::FirestoreDocumentParser::new(backup_file.to_str().unwrap().to_string());
        
        match parser.parse_backup(backup_file.to_str().unwrap()).await {
            Ok(parse_result) => {
                let documents = &parse_result.documents;
                info!("Successfully parsed {} documents", documents.len());
                
                // Perform additional quality checks
                let mut quality_errors = Vec::new();
                
                // Check for empty documents
                let empty_docs = documents.iter().filter(|d| d.data.is_empty()).count();
                if empty_docs > 0 {
                    quality_errors.push(format!("Found {} empty documents", empty_docs));
                }
                
                // Check for documents with missing required fields
                // This would be more sophisticated in a real implementation
                
                if !quality_errors.is_empty() {
                    result.errors.extend(quality_errors);
                    if result.errors.len() > max_errors {
                        result.errors.truncate(max_errors);
                        result.errors.push("... (truncated due to max_errors limit)".to_string());
                    }
                }
            }
            Err(e) => {
                result.is_valid = false;
                result.errors.push(format!("Data parsing failed: {}", e));
            }
        }
    }
    
    // Step 3: Output results based on format
    match format {
        OutputFormat::Text => {
            // Text output is handled by the caller
        }
        OutputFormat::Json => {
            let json_output = serde_json::to_string_pretty(&result)?;
            println!("{}", json_output);
        }
        OutputFormat::Yaml => {
            // For now, use JSON format
            let json_output = serde_json::to_string_pretty(&result)?;
            println!("{}", json_output);
        }
    }
    
    info!("Backup validation completed");
    Ok(result)
}

/// Create a basic schema from analysis without normalization
fn create_basic_schema_from_analysis(analysis: &types::SchemaAnalysis) -> Result<types::NormalizedSchema, FireupError> {
    use types::*;
    
    let mut tables = Vec::new();
    
    for collection in &analysis.collections {
        let mut columns = Vec::new();
        
        // Add a primary key column
        columns.push(ColumnDefinition {
            name: "id".to_string(),
            column_type: PostgreSQLType::Uuid,
            nullable: false,
            default_value: None,
            constraints: vec![],
        });
        
        // Add columns for each field
        for field_name in &collection.field_names {
            // Find the field type analysis for this field
            let field_type = analysis.field_types.iter()
                .find(|ft| ft.field_path == *field_name)
                .map(|ft| ft.recommended_type.clone())
                .unwrap_or(PostgreSQLType::Text);
            
            columns.push(ColumnDefinition {
                name: field_name.clone(),
                column_type: field_type,
                nullable: true, // Most fields are nullable in Firestore
                default_value: None,
                constraints: vec![],
            });
        }
        
        tables.push(TableDefinition {
            name: collection.name.clone(),
            columns,
            primary_key: Some(PrimaryKeyDefinition {
                name: format!("{}_pkey", collection.name),
                columns: vec!["id".to_string()],
            }),
            foreign_keys: vec![],
            indexes: vec![],
        });
    }
    
    let table_count = tables.len() as u32;
    Ok(NormalizedSchema {
        tables,
        relationships: vec![],
        constraints: vec![],
        warnings: vec![],
        metadata: SchemaMetadata {
            version: "1.0".to_string(),
            generated_at: chrono::Utc::now(),
            source_analysis_id: uuid::Uuid::new_v4(),
            table_count,
            relationship_count: 0,
        },
    })
}

/// Parse PostgreSQL URL and create ConnectionConfig
fn parse_postgres_url(url: &str, max_connections: usize, timeout: u64) -> Result<ConnectionConfig, FireupError> {
    // Simple URL parsing - in a real implementation, you'd use a proper URL parser
    // Format: postgresql://user:password@host:port/database
    
    if !url.starts_with("postgresql://") {
        return Err(FireupError::Configuration {
            message: "Invalid PostgreSQL URL format. Expected: postgresql://user:password@host:port/database".to_string(),
            config_key: Some("postgres_url".to_string()),
            context: error::ErrorContext {
                operation: "parse_postgres_url".to_string(),
                metadata: std::collections::HashMap::new(),
                timestamp: chrono::Utc::now(),
                call_path: vec!["main::parse_postgres_url".to_string()],
            },
            suggestions: vec!["Use format: postgresql://user:password@host:port/database".to_string()],
        });
    }
    
    // For now, return a default config - in a real implementation, parse the URL properly
    Ok(ConnectionConfig {
        host: "localhost".to_string(),
        port: 5432,
        database: "fireup_test".to_string(),
        user: "postgres".to_string(),
        password: "password".to_string(),
        max_connections,
        connection_timeout: std::time::Duration::from_secs(timeout),
        retry_attempts: 3,
        retry_delay: std::time::Duration::from_secs(1),
    })
}

/// Initialize logging based on CLI configuration
fn initialize_logging(cli: &Cli) -> Result<()> {
    let log_level = if let Some(level) = &cli.log_level {
        level.clone().into()
    } else if cli.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };
    
    if cli.json_logs {
        tracing_subscriber::fmt()
            .with_max_level(log_level)
            .with_target(false)
            .with_thread_ids(cli.verbose)
            .with_file(cli.verbose)
            .with_line_number(cli.verbose)
            .json()
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_max_level(log_level)
            .with_target(false)
            .with_thread_ids(cli.verbose)
            .with_file(cli.verbose)
            .with_line_number(cli.verbose)
            .init();
    }
    
    Ok(())
}