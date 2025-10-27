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
    
    /// Show system monitoring statistics and performance metrics
    #[command(long_about = "Display comprehensive monitoring statistics including operation metrics, performance data, and audit log summaries. This command helps track system performance and troubleshoot issues.")]
    Stats {
        /// Show detailed performance metrics
        #[arg(long)]
        detailed: bool,
        
        /// Filter operations by name pattern
        #[arg(long)]
        operation_filter: Option<String>,
        
        /// Number of recent audit entries to show (default: 10)
        #[arg(long, default_value = "10")]
        audit_entries: usize,
        
        /// Output format for statistics
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
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
    let monitoring_config = monitoring::MonitoringConfig {
        enable_performance_tracking: true,
        enable_audit_logging: true,
        max_completed_operations: 1000,
        max_audit_entries: 10000,
        min_tracking_duration_ms: if cli.verbose { 10 } else { 100 },
    };
    monitoring::initialize_monitoring(monitoring_config);
    
    info!("Starting Fireup migration tool v{}", env!("CARGO_PKG_VERSION"));
    info!("Monitoring and audit logging enabled");
    
    match cli.command {
        Commands::Import { 
            backup_file, 
            postgres_url: _postgres_url, 
            batch_size,
            max_connections,
            skip_normalization,
            drop_existing,
            continue_on_error,
            generate_ddl,
            timeout,
        } => {
            let tracker = monitoring::get_monitoring_system().start_operation("cli_import_command").await;
            
            info!("Starting import from {:?} to PostgreSQL", backup_file);
            info!("Configuration: batch_size={}, max_connections={}, skip_normalization={}, drop_existing={}, continue_on_error={}, timeout={}s", 
                  batch_size, max_connections, skip_normalization, drop_existing, continue_on_error, timeout);
            
            if let Some(ddl_path) = &generate_ddl {
                info!("Will generate DDL file at: {:?}", ddl_path);
            }
            
            // Log command execution
            let mut details = std::collections::HashMap::new();
            details.insert("backup_file".to_string(), backup_file.display().to_string());
            details.insert("batch_size".to_string(), batch_size.to_string());
            details.insert("max_connections".to_string(), max_connections.to_string());
            details.insert("skip_normalization".to_string(), skip_normalization.to_string());
            
            monitoring::get_monitoring_system().log_audit_entry(
                monitoring::AuditOperationType::SystemConfiguration,
                "cli_command",
                "import",
                "command_executed",
                monitoring::AuditResult::Success,
                details,
                None,
            ).await.ok();
            
            // TODO: Implement import functionality in task 8.2
            println!("Import functionality will be implemented in task 8.2");
            
            tracker.complete_success().await.ok();
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
            let tracker = monitoring::get_monitoring_system().start_operation("cli_analyze_command").await;
            
            info!("Analyzing schema from {:?}", backup_file);
            info!("Configuration: normalize={}, generate_indexes={}, detailed={}, format={:?}, show_conflicts={}", 
                  normalize, generate_indexes, detailed, format, show_conflicts);
            
            let output_path = output.unwrap_or_else(|| PathBuf::from("schema.sql"));
            info!("Output will be written to: {:?}", output_path);
            
            // Log command execution
            let mut details = std::collections::HashMap::new();
            details.insert("backup_file".to_string(), backup_file.display().to_string());
            details.insert("output_path".to_string(), output_path.display().to_string());
            details.insert("normalize".to_string(), normalize.to_string());
            details.insert("generate_indexes".to_string(), generate_indexes.to_string());
            details.insert("detailed".to_string(), detailed.to_string());
            
            monitoring::get_monitoring_system().log_audit_entry(
                monitoring::AuditOperationType::DataAccess,
                "cli_command",
                "analyze",
                "command_executed",
                monitoring::AuditResult::Success,
                details,
                None,
            ).await.ok();
            
            // TODO: Implement analyze functionality in task 8.2
            println!("Analyze functionality will be implemented in task 8.2");
            
            tracker.complete_success().await.ok();
        }
        Commands::Validate { 
            backup_file, 
            detailed,
            check_quality,
            format,
            max_errors,
        } => {
            let tracker = monitoring::get_monitoring_system().start_operation("cli_validate_command").await;
            
            info!("Validating backup file {:?}", backup_file);
            info!("Configuration: detailed={}, check_quality={}, format={:?}, max_errors={}", 
                  detailed, check_quality, format, max_errors);
            
            // Log command execution
            let mut details = std::collections::HashMap::new();
            details.insert("backup_file".to_string(), backup_file.display().to_string());
            details.insert("detailed".to_string(), detailed.to_string());
            details.insert("check_quality".to_string(), check_quality.to_string());
            details.insert("max_errors".to_string(), max_errors.to_string());
            
            monitoring::get_monitoring_system().log_audit_entry(
                monitoring::AuditOperationType::DataAccess,
                "cli_command",
                "validate",
                "command_executed",
                monitoring::AuditResult::Success,
                details,
                None,
            ).await.ok();
            
            // TODO: Implement validate functionality in task 8.2
            println!("Validate functionality will be implemented in task 8.2");
            
            tracker.complete_success().await.ok();
        }
        
        Commands::Stats {
            detailed,
            operation_filter,
            audit_entries,
            format,
        } => {
            let tracker = monitoring::get_monitoring_system().start_operation("cli_stats_command").await;
            
            info!("Displaying system monitoring statistics");
            
            // Get system statistics
            let stats = monitoring::get_monitoring_system().get_system_stats().await;
            
            match format {
                OutputFormat::Json => {
                    let json_output = serde_json::to_string_pretty(&stats)
                        .unwrap_or_else(|_| "Error serializing stats".to_string());
                    println!("{}", json_output);
                }
                OutputFormat::Yaml => {
                    // For now, output as JSON since we don't have yaml dependency
                    let json_output = serde_json::to_string_pretty(&stats)
                        .unwrap_or_else(|_| "Error serializing stats".to_string());
                    println!("{}", json_output);
                }
                OutputFormat::Text => {
                    println!("=== Fireup System Statistics ===");
                    println!("Active Operations: {}", stats.active_operations);
                    println!("Completed Operations: {}", stats.completed_operations);
                    println!("Total Operations: {}", stats.total_operations);
                    println!("Successful Operations: {}", stats.successful_operations);
                    println!("Failed Operations: {}", stats.failed_operations);
                    println!("Average Duration: {:.2} ms", stats.avg_duration_ms);
                    println!("Total Records Processed: {}", stats.total_records_processed);
                    println!("Audit Entries: {}", stats.audit_entries);
                    
                    if detailed {
                        println!("\n=== Performance Metrics ===");
                        let metrics = monitoring::get_monitoring_system()
                            .get_performance_metrics(operation_filter.as_deref())
                            .await;
                        
                        for metric in metrics.iter().take(10) {
                            println!("Operation: {} ({})", metric.operation_name, metric.operation_id);
                            println!("  Status: {:?}", metric.status);
                            println!("  Duration: {:?} ms", metric.duration_ms);
                            println!("  Records: {:?}", metric.records_processed);
                            println!("  Throughput: {:?} records/sec", metric.throughput);
                            println!();
                        }
                    }
                    
                    println!("\n=== Recent Audit Entries ===");
                    let recent_entries = monitoring::get_monitoring_system()
                        .get_recent_audit_entries(audit_entries)
                        .await;
                    
                    for entry in recent_entries {
                        println!("{} - {} {} on {} ({})", 
                            entry.timestamp.format("%Y-%m-%d %H:%M:%S UTC"),
                            entry.action,
                            entry.resource_type,
                            entry.resource_id,
                            match entry.result {
                                monitoring::AuditResult::Success => "SUCCESS".to_string(),
                                monitoring::AuditResult::Failure(ref e) => format!("FAILED: {}", e),
                                monitoring::AuditResult::PartialSuccess(ref e) => format!("PARTIAL: {}", e),
                            }
                        );
                    }
                }
            }
            
            // Log stats command execution
            let mut details = std::collections::HashMap::new();
            details.insert("detailed".to_string(), detailed.to_string());
            details.insert("audit_entries_requested".to_string(), audit_entries.to_string());
            if let Some(filter) = &operation_filter {
                details.insert("operation_filter".to_string(), filter.clone());
            }
            
            monitoring::get_monitoring_system().log_audit_entry(
                monitoring::AuditOperationType::DataAccess,
                "cli_command",
                "stats",
                "command_executed",
                monitoring::AuditResult::Success,
                details,
                None,
            ).await.ok();
            
            tracker.complete_success().await.ok();
        }
    }
    
    Ok(())
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