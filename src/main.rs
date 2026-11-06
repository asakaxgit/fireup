use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use tracing::{info, warn, Level};
use tracing_subscriber;
use std::path::PathBuf;

mod error;
mod types;
mod protos;
mod leveldb_parser;
mod schema_analyzer;
mod data_importer;
mod monitoring;

// The local data plane is consumed from the library crate rather than
// re-declared here, so there is exactly one copy of it in the build.
use fireup::local;

use error::FireupError;
use leveldb_parser::{LevelDBParser, BackupValidatorImpl, ValidationResult};
use leveldb_parser::validator::BackupValidator;
use schema_analyzer::{DocumentStructureAnalyzer, NormalizationEngine, DDLGenerator};
use data_importer::{
    parse_connection_url, DocumentTransformer, FullImportResult, PostgreSQLImporter, TableImportSpec,
};
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
        
        /// Continue import on constraint violations: import the remaining rows
        /// instead of aborting. The exit code still reports the failures, since a
        /// partial import is not a completed migration.
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

    /// Run a local Firestore-, Datastore-, and Firebase-Auth-compatible server
    #[command(long_about = "Serve the public google.firestore.v1 and google.datastore.v1 gRPC APIs plus a subset of the public Identity Toolkit (Firebase Auth) REST API on a single local port, so official client SDKs can talk to Fireup via the documented FIRESTORE_EMULATOR_HOST, DATASTORE_EMULATOR_HOST, and FIREBASE_AUTH_EMULATOR_HOST environment variables. The same port answers HTTP GET / (service discovery) and GET /health.

    export FIRESTORE_EMULATOR_HOST=127.0.0.1:4589
    export DATASTORE_EMULATOR_HOST=127.0.0.1:4589
    export FIREBASE_AUTH_EMULATOR_HOST=127.0.0.1:4589

This is an independent implementation of Google's published APIs. It is not the Firebase Emulator Suite and is not affiliated with Google.

FIRESTORE: document CRUD, StructuredQuery, collection listings, and transactions. Realtime listeners, field transforms, and aggregations return UNIMPLEMENTED.

DATASTORE: Lookup, RunQuery, Commit, BeginTransaction, AllocateIds on the same store as Firestore (kinds map to collections). GQL, aggregations, namespaces, and property transforms return UNIMPLEMENTED.

AUTH: email/password sign-up and sign-in, anonymous sign-in, custom-token sign-in, token refresh, admin account CRUD, and the email flows (password reset, email verification, email-link sign-in). No mail is sent: the link is logged and readable at GET /emulator/v1/projects/{p}/oobCodes. Phone auth, multi-factor enrolment, and federated providers are not served.

Storage is in-memory unless --postgres-url is given, in which case documents and accounts are stored in PostgreSQL and survive a restart.

LOCAL DEVELOPMENT ONLY: the auth surface mints unsigned ID tokens and checks no API key or admin credential, so anything that can reach the port can act as any account. The default bind is loopback; keep it that way.")]
    Local {
        #[command(subcommand)]
        command: Option<LocalCommands>,

        /// Address to bind (default: loopback only)
        #[arg(long, default_value = "127.0.0.1", global = true)]
        host: String,

        /// Port to listen on
        #[arg(short, long, default_value_t = local::DEFAULT_PORT, global = true)]
        port: u16,

        /// Store documents and auth accounts in PostgreSQL instead of memory,
        /// so they survive a restart
        /// (format: postgresql://user:password@host:port/database)
        #[arg(long, value_name = "URL", global = true)]
        postgres_url: Option<String>,

        /// Project id this data plane answers for
        #[arg(long, default_value = local::LocalConfig::DEFAULT_PROJECT_ID, global = true)]
        project_id: String,

        /// Database id; Firestore's default database is literally "(default)".
        /// Datastore clients omit the id (empty string) for the default database;
        /// both forms address the same store.
        #[arg(long, default_value = local::LocalConfig::DEFAULT_DATABASE_ID, global = true)]
        database_id: String,

        /// Services to enable (repeatable); all three are on by default
        #[arg(
            long = "enable",
            value_enum,
            default_values_t = [
                local::Service::Firestore,
                local::Service::Datastore,
                local::Service::Auth,
            ],
            global = true
        )]
        enable: Vec<local::Service>,

        /// PostgreSQL connection pool size
        #[arg(long, default_value_t = local::LocalConfig::DEFAULT_MAX_CONNECTIONS, global = true)]
        max_connections: usize,
    },
}

#[derive(Subcommand)]
enum LocalCommands {
    /// Load a managed Firestore/Datastore export into the local store
    #[command(long_about = "Parse a managed export with the same parser `fireup import` uses and write every document into the local data plane's store.

Seeding upserts, so running it twice is the same as running it once. Pass --postgres-url to seed a durable store; without it the documents go into an in-memory store that disappears when the command exits, which is only useful as a dry run.")]
    Seed {
        /// Path to the export file or directory
        #[arg(short, long, value_name = "FILE")]
        backup_file: PathBuf,
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
                    info!("Summary: {}", result.summary());
                    if result.is_successful() {
                        info!("Import completed successfully!");
                    } else {
                        eprintln!("Import did not complete successfully: {}", result.summary());
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
        Commands::Local {
            command,
            host,
            port,
            postgres_url,
            project_id,
            database_id,
            enable,
            max_connections,
        } => {
            let addr: std::net::SocketAddr = format!("{host}:{port}")
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid bind address {host}:{port}: {e}"))?;

            let mut config = local::LocalConfig::in_memory(addr)
                .with_project_id(project_id)
                .with_database_id(database_id);
            config.postgres_url = postgres_url;
            config.max_connections = max_connections;
            // clap keeps the default alongside repeated values, so de-duplicate
            // rather than enabling the same service twice.
            config.services = {
                let mut services = enable;
                services.sort();
                services.dedup();
                services
            };

            let outcome = match command {
                None => local::serve(config).await,
                Some(LocalCommands::Seed { backup_file }) => {
                    execute_local_seed(config, &backup_file).await
                }
            };

            if let Err(e) = outcome {
                eprintln!("fireup local failed: {e:#}");
                std::process::exit(1);
            }
        }
    }
    
    Ok(())
}

/// Seed the local data plane's store from a managed export.
///
/// A seed that could not write every parsed document is a failure: the caller
/// asked for the export to be in the store, and a partially seeded store looks
/// identical to a complete one from the outside.
async fn execute_local_seed(
    config: local::LocalConfig,
    backup_file: &PathBuf,
) -> Result<()> {
    config.validate()?;
    let root = config.documents_root();
    info!("Seeding {} from {:?}", root, backup_file);

    if config.postgres_url.is_none() {
        warn!(
            "no --postgres-url given: seeding an in-memory store that is discarded when this \
             command exits (dry run)"
        );
    }

    let store = local::build_store(&config).await?;
    let report = local::seed::seed_from_export(&store, &root, backup_file).await?;

    info!("Seed complete: {}", report.summary());
    info!("Store now holds {} documents", store.count().await?);

    if !report.is_complete() {
        for skip in &report.skipped {
            eprintln!("  skipped {}: {}", skip.source, skip.reason);
        }
        anyhow::bail!(
            "{} of {} documents were not seeded; the store does not hold the whole export",
            report.skipped.len(),
            report.parsed
        );
    }

    // An export that parsed to nothing is already caught by the parser guard in
    // `ensure_documents_parsed`; here it means an empty source.
    if report.parsed == 0 {
        warn!("the export held no documents; nothing was seeded");
    }

    Ok(())
}

/// Execute the complete import pipeline from backup file to PostgreSQL
async fn execute_import_pipeline(
    backup_file: &PathBuf,
    postgres_url: &str,
    batch_size: usize,
    max_connections: usize,
    skip_normalization: bool,
    drop_existing: bool,
    continue_on_error: bool,
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
    ensure_documents_parsed(backup_file, documents.len())?;

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
    let connection_config = parse_connection_url(postgres_url, max_connections, timeout)?;
    info!(
        "Connecting to {}:{}/{} as {}",
        connection_config.host, connection_config.port, connection_config.database, connection_config.user
    );

    let importer = PostgreSQLImporter::new(connection_config).await?;
    info!("PostgreSQL connection established");

    // Step 6: Transform documents into table rows
    info!("Step 6: Transforming documents into table rows");
    let mut transformer = DocumentTransformer::new();
    let transformation_result = transformer.transform_documents(documents, &normalized_schema)?;
    info!(
        "Transformed {} documents into {} rows across {} tables",
        transformation_result.statistics.documents_processed,
        transformation_result.statistics.total_rows,
        transformation_result.table_data.len()
    );
    for warning in &transformation_result.warnings {
        warn!("Transformation warning: {}", warning);
    }

    let table_specs = build_table_import_specs(
        &normalized_schema,
        &transformation_result,
        batch_size,
        continue_on_error,
    );

    // Step 7: Drop existing tables (if requested)
    let ddl_generator = DDLGenerator::new();
    let generated_ddl = ddl_generator.generate_ddl(&normalized_schema)?;
    let ddl_statements = generated_ddl.all_statements();

    if drop_existing {
        let table_names: Vec<String> = normalized_schema
            .tables
            .iter()
            .map(|table| table.name.clone())
            .collect();
        info!("Step 7: Dropping {} existing tables as requested", table_names.len());
        let drop_result = importer.drop_tables(&table_names).await?;
        info!("Dropped {} existing tables", drop_result.imported_records);
    } else {
        info!("Step 7: Keeping existing tables (--drop-existing not set)");
    }

    // Step 8: Create the schema and import every table's rows
    info!("Step 8: Creating schema and importing data");
    let full_result = importer.execute_full_import(&ddl_statements, &table_specs).await?;

    for warning in &full_result.warnings {
        warn!("Import warning: {}", warning);
    }

    // A zero-row import of a non-empty source is a failed migration, not a
    // successful one: refuse to report success for it.
    if full_result.total_records_expected > 0 && full_result.total_records_imported == 0 {
        return Err(FireupError::data_import(
            format!(
                "Imported 0 of {} expected rows — no data reached PostgreSQL",
                full_result.total_records_expected
            ),
            None,
            None,
            FireupError::new_context("execute_import_pipeline"),
        ));
    }

    if full_result.total_records_failed > 0 && !continue_on_error {
        return Err(FireupError::data_import(
            format!(
                "{} of {} rows failed to import. Re-run with --continue-on-error to import the remainder",
                full_result.total_records_failed, full_result.total_records_expected
            ),
            None,
            None,
            FireupError::new_context("execute_import_pipeline"),
        ));
    }

    info!("Import pipeline completed: {}", full_result.summary());
    Ok(full_result)
}

/// Build one import spec per table that the transformation produced rows for.
fn build_table_import_specs(
    schema: &types::NormalizedSchema,
    transformation: &data_importer::TransformationResult,
    batch_size: usize,
    continue_on_error: bool,
) -> Vec<TableImportSpec> {
    schema
        .tables
        .iter()
        .filter_map(|table| {
            let rows = transformation.table_data.get(&table.name)?;
            if rows.is_empty() {
                info!("Table '{}' has no rows to import", table.name);
                return None;
            }
            Some(
                TableImportSpec::from_table_rows(table, rows)
                    .with_batch_size(batch_size)
                    .with_continue_on_error(continue_on_error),
            )
        })
        .collect()
}

/// Total size in bytes of a backup path, following one level of export tree.
fn source_size_bytes(path: &PathBuf) -> u64 {
    fn walk(path: &std::path::Path, depth: usize) -> u64 {
        let Ok(metadata) = fs::metadata(path) else { return 0 };
        if metadata.is_file() {
            return metadata.len();
        }
        if !metadata.is_dir() || depth == 0 {
            return 0;
        }
        let Ok(entries) = fs::read_dir(path) else { return 0 };
        entries
            .filter_map(Result::ok)
            .map(|entry| walk(&entry.path(), depth - 1))
            .sum()
    }

    walk(path, 8)
}

/// Fail when a non-empty backup source yielded no documents at all.
///
/// Total parse loss used to exit 0 with "completed successfully", which is
/// indistinguishable from a backup that legitimately holds no documents.
fn ensure_documents_parsed(backup_file: &PathBuf, document_count: usize) -> Result<(), FireupError> {
    if document_count > 0 {
        return Ok(());
    }

    let source_bytes = source_size_bytes(backup_file);
    if source_bytes == 0 {
        warn!("Backup source {:?} is empty; nothing to do", backup_file);
        return Ok(());
    }

    Err(FireupError::leveldb_parse(
        format!(
            "Parsed 0 documents from {:?} ({} bytes of input). Refusing to continue: the source is not empty, \
             so this is a parse failure rather than an empty backup",
            backup_file, source_bytes
        ),
        FireupError::new_context("ensure_documents_parsed"),
    ))
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
    ensure_documents_parsed(backup_file, documents.len())?;

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

                // Readable records that decode to nothing must not pass quality checks.
                if documents.is_empty() && parse_result.metadata.records_processed > 0 {
                    quality_errors.push(format!(
                        "Decoded 0 Firestore documents from {} valid LevelDB records",
                        parse_result.metadata.records_processed
                    ));
                    result.integrity_info.overall_integrity_score = 0.0;
                    result.integrity_info.parsing_errors = result
                        .integrity_info
                        .parsing_errors
                        .saturating_add(parse_result.metadata.records_processed);
                }

                // Check for empty documents
                let empty_docs = documents.iter().filter(|d| d.data.is_empty()).count();
                if empty_docs > 0 {
                    quality_errors.push(format!("Found {} empty documents", empty_docs));
                }

                if !quality_errors.is_empty() {
                    result.is_valid = false;
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