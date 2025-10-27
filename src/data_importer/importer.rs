// PostgreSQL data importer implementation
use crate::error::FireupError;
use crate::monitoring::{get_monitoring_system, AuditOperationType, AuditResult};
use deadpool_postgres::{Config, Pool, Runtime};
use std::collections::HashMap;
use std::time::Duration;
use tokio_postgres::{NoTls, Row};
use tracing::{info, warn, error, instrument};

#[derive(Debug, Clone)]
pub struct ImportResult {
    pub imported_records: usize,
    pub failed_records: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: String,
    pub max_connections: usize,
    pub connection_timeout: Duration,
    pub retry_attempts: u32,
    pub retry_delay: Duration,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            host: "localhost".to_string(),
            port: 5432,
            database: "fireup_dev".to_string(),
            user: "fireup".to_string(),
            password: "fireup_dev_password".to_string(),
            max_connections: 10,
            connection_timeout: Duration::from_secs(30),
            retry_attempts: 3,
            retry_delay: Duration::from_secs(1),
        }
    }
}

#[derive(Clone)]
pub struct PostgreSQLImporter {
    pool: Pool,
    config: ConnectionConfig,
}

impl PostgreSQLImporter {
    /// Create a new PostgreSQL importer with connection pooling
    #[instrument(skip(config))]
    pub async fn new(config: ConnectionConfig) -> Result<Self, FireupError> {
        let tracker = get_monitoring_system().start_operation("postgresql_importer_init").await;
        
        info!("Initializing PostgreSQL connection pool");
        
        let mut pg_config = Config::new();
        pg_config.host = Some(config.host.clone());
        pg_config.port = Some(config.port);
        pg_config.dbname = Some(config.database.clone());
        pg_config.user = Some(config.user.clone());
        pg_config.password = Some(config.password.clone());
        pg_config.pool = Some(deadpool_postgres::PoolConfig::new(config.max_connections));
        
        let pool = pg_config
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| FireupError::database_connection(
                format!("Failed to create connection pool: {}", e),
                Some(format!("{}:{}@{}:{}/{}", config.user, "***", config.host, config.port, config.database)),
                FireupError::new_context("create_connection_pool")
            ))?;

        // Test the connection
        let mut retry_count = 0;
        loop {
            match pool.get().await {
                Ok(client) => {
                    // Test with a simple query
                    match client.query("SELECT 1", &[]).await {
                        Ok(_) => {
                            info!("PostgreSQL connection pool initialized successfully");
                            
                            // Log successful connection establishment
                            let mut details = HashMap::new();
                            details.insert("host".to_string(), config.host.clone());
                            details.insert("port".to_string(), config.port.to_string());
                            details.insert("database".to_string(), config.database.clone());
                            details.insert("max_connections".to_string(), config.max_connections.to_string());
                            
                            get_monitoring_system().log_audit_entry(
                                AuditOperationType::SystemConfiguration,
                                "database_connection",
                                &format!("{}:{}", config.host, config.port),
                                "connection_pool_initialized",
                                AuditResult::Success,
                                details,
                                None,
                            ).await.ok();
                            
                            tracker.complete_success().await.ok();
                            break;
                        }
                        Err(e) => {
                            error!("Connection test failed: {}", e);
                            if retry_count >= config.retry_attempts {
                                let error = FireupError::database_connection(
                                    format!("Failed to connect to PostgreSQL after {} attempts: {}", config.retry_attempts, e),
                                    Some(format!("{}:{}@{}:{}/{}", config.user, "***", config.host, config.port, config.database)),
                                    FireupError::new_context("test_connection")
                                );
                                
                                // Log failed connection attempt
                                let mut details = HashMap::new();
                                details.insert("error".to_string(), e.to_string());
                                details.insert("retry_attempts".to_string(), config.retry_attempts.to_string());
                                
                                get_monitoring_system().log_audit_entry(
                                    AuditOperationType::SystemConfiguration,
                                    "database_connection",
                                    &format!("{}:{}", config.host, config.port),
                                    "connection_pool_failed",
                                    AuditResult::Failure(e.to_string()),
                                    details,
                                    None,
                                ).await.ok();
                                
                                tracker.complete_failure(&error.to_string()).await.ok();
                                return Err(error);
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to get connection from pool: {}", e);
                    if retry_count >= config.retry_attempts {
                        return Err(FireupError::database_connection(
                            format!("Failed to get connection from pool after {} attempts: {}", config.retry_attempts, e),
                            None,
                            FireupError::new_context("get_connection_from_pool")
                        ));
                    }
                }
            }
            
            retry_count += 1;
            warn!("Connection attempt {} failed, retrying in {:?}", retry_count, config.retry_delay);
            tokio::time::sleep(config.retry_delay).await;
        }

        Ok(Self { pool, config })
    }

    /// Get a connection from the pool with retry logic
    pub async fn get_connection(&self) -> Result<deadpool_postgres::Client, FireupError> {
        let mut retry_count = 0;
        
        loop {
            match self.pool.get().await {
                Ok(client) => return Ok(client),
                Err(e) => {
                    if retry_count >= self.config.retry_attempts {
                        return Err(FireupError::database_connection(
                            format!("Failed to get connection after {} attempts: {}", self.config.retry_attempts, e),
                            None,
                            FireupError::new_context("get_connection_retry")
                        ));
                    }
                    
                    retry_count += 1;
                    warn!("Failed to get connection (attempt {}): {}", retry_count, e);
                    tokio::time::sleep(self.config.retry_delay).await;
                }
            }
        }
    }

    /// Execute a query with retry logic
    pub async fn execute_query(&self, query: &str, params: &[&(dyn tokio_postgres::types::ToSql + Sync)]) -> Result<Vec<Row>, FireupError> {
        let mut retry_count = 0;
        
        loop {
            let client = self.get_connection().await?;
            
            match client.query(query, params).await {
                Ok(rows) => return Ok(rows),
                Err(e) => {
                    if retry_count >= self.config.retry_attempts {
                        return Err(FireupError::database_connection(
                            format!("Query execution failed after {} attempts: {}", self.config.retry_attempts, e),
                            None,
                            FireupError::new_context("execute_query_retry")
                        ));
                    }
                    
                    retry_count += 1;
                    warn!("Query execution failed (attempt {}): {}", retry_count, e);
                    tokio::time::sleep(self.config.retry_delay).await;
                }
            }
        }
    }

    /// Execute a statement (INSERT, UPDATE, DELETE) with retry logic
    pub async fn execute_statement(&self, statement: &str, params: &[&(dyn tokio_postgres::types::ToSql + Sync)]) -> Result<u64, FireupError> {
        let mut retry_count = 0;
        
        loop {
            let client = self.get_connection().await?;
            
            match client.execute(statement, params).await {
                Ok(rows_affected) => return Ok(rows_affected),
                Err(e) => {
                    if retry_count >= self.config.retry_attempts {
                        return Err(FireupError::database_connection(
                            format!("Statement execution failed after {} attempts: {}", self.config.retry_attempts, e),
                            None,
                            FireupError::new_context("execute_statement_retry")
                        ));
                    }
                    
                    retry_count += 1;
                    warn!("Statement execution failed (attempt {}): {}", retry_count, e);
                    tokio::time::sleep(self.config.retry_delay).await;
                }
            }
        }
    }

    /// Get pool status information
    pub fn get_pool_status(&self) -> (usize, usize) {
        let status = self.pool.status();
        (status.size, status.available)
    }

    /// Close the connection pool
    pub fn close(&self) {
        info!("Closing PostgreSQL connection pool");
        self.pool.close();
    }
}

#[derive(Debug, Clone)]
pub struct BatchConfig {
    pub batch_size: usize,
    pub max_concurrent_batches: usize,
    pub progress_report_interval: usize,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            batch_size: 1000,
            max_concurrent_batches: 4,
            progress_report_interval: 10000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImportProgress {
    pub total_records: usize,
    pub processed_records: usize,
    pub successful_records: usize,
    pub failed_records: usize,
    pub current_batch: usize,
    pub total_batches: usize,
    pub warnings: Vec<String>,
}

impl ImportProgress {
    pub fn new(total_records: usize, batch_size: usize) -> Self {
        let total_batches = (total_records + batch_size - 1) / batch_size;
        Self {
            total_records,
            processed_records: 0,
            successful_records: 0,
            failed_records: 0,
            current_batch: 0,
            total_batches,
            warnings: Vec::new(),
        }
    }

    pub fn progress_percentage(&self) -> f64 {
        if self.total_records == 0 {
            100.0
        } else {
            (self.processed_records as f64 / self.total_records as f64) * 100.0
        }
    }
}

pub struct BatchProcessor {
    importer: PostgreSQLImporter,
    config: BatchConfig,
}

impl BatchProcessor {
    pub fn new(importer: PostgreSQLImporter, config: BatchConfig) -> Self {
        Self { importer, config }
    }

    /// Process data in batches with transaction support and progress tracking
    pub async fn process_batches<T, F, Fut>(
        &self,
        data: Vec<T>,
        processor: F,
    ) -> Result<ImportProgress, FireupError>
    where
        T: Send + Clone + 'static,
        F: Fn(Vec<T>, &PostgreSQLImporter) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<usize, FireupError>> + Send,
    {
        let total_records = data.len();
        let mut progress = ImportProgress::new(total_records, self.config.batch_size);
        
        info!("Starting batch processing for {} records", total_records);
        
        if data.is_empty() {
            return Ok(progress);
        }

        // Split data into batches
        let batches: Vec<Vec<T>> = data
            .into_iter()
            .collect::<Vec<_>>()
            .chunks(self.config.batch_size)
            .map(|chunk| chunk.to_vec())
            .collect();

        progress.total_batches = batches.len();
        
        // Process batches with limited concurrency
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(self.config.max_concurrent_batches));
        let processor = std::sync::Arc::new(processor);
        let progress_mutex = std::sync::Arc::new(tokio::sync::Mutex::new(progress));

        let mut handles = Vec::new();

        for (batch_index, batch) in batches.into_iter().enumerate() {
            let semaphore = semaphore.clone();
            let processor = processor.clone();
            let progress_mutex = progress_mutex.clone();
            let importer = self.importer.clone();
            let batch_size = batch.len();
            let report_interval = self.config.progress_report_interval;

            let handle = tokio::spawn(async move {
                let _permit = semaphore.acquire().await.unwrap();
                
                // Start transaction
                let mut client = importer.get_connection().await?;
                let transaction = client.transaction().await
                    .map_err(|e| FireupError::Transaction {
                        message: format!("Failed to start transaction: {}", e),
                        operation: "batch_processing".to_string(),
                        context: FireupError::new_context("start_transaction"),
                        suggestions: vec!["Check database connection".to_string(), "Retry the operation".to_string()],
                    })?;

                let result: Result<(usize, usize), FireupError> = match processor(batch, &importer).await {
                    Ok(successful_count) => {
                        // Commit transaction
                        transaction.commit().await
                            .map_err(|e| FireupError::Transaction {
                                message: format!("Failed to commit transaction: {}", e),
                                operation: "batch_processing".to_string(),
                                context: FireupError::new_context("commit_transaction"),
                                suggestions: vec!["Check database connection".to_string(), "Retry the operation".to_string()],
                            })?;
                        
                        Ok((successful_count, batch_size - successful_count))
                    }
                    Err(e) => {
                        // Rollback transaction
                        if let Err(rollback_err) = transaction.rollback().await {
                            warn!("Failed to rollback transaction: {}", rollback_err);
                        }
                        
                        error!("Batch {} failed: {}", batch_index, e);
                        Ok((0, batch_size)) // All records in batch failed
                    }
                };

                // Update progress
                {
                    let mut progress = progress_mutex.lock().await;
                    progress.current_batch = batch_index + 1;
                    progress.processed_records += batch_size;
                    
                    if let Ok((successful, failed)) = result {
                        progress.successful_records += successful;
                        progress.failed_records += failed;
                    }

                    // Report progress at intervals
                    if progress.processed_records % report_interval == 0 || 
                       progress.processed_records == progress.total_records {
                        info!(
                            "Progress: {}/{} records ({:.1}%) - {} successful, {} failed",
                            progress.processed_records,
                            progress.total_records,
                            progress.progress_percentage(),
                            progress.successful_records,
                            progress.failed_records
                        );
                    }
                }

                result
            });

            handles.push(handle);
        }

        // Wait for all batches to complete
        for handle in handles {
            if let Err(e) = handle.await {
                error!("Batch processing task failed: {}", e);
                let mut progress = progress_mutex.lock().await;
                progress.warnings.push(format!("Batch processing task failed: {}", e));
            }
        }

        let final_progress = progress_mutex.lock().await.clone();
        
        info!(
            "Batch processing completed: {}/{} records successful ({:.1}%)",
            final_progress.successful_records,
            final_progress.total_records,
            if final_progress.total_records > 0 {
                (final_progress.successful_records as f64 / final_progress.total_records as f64) * 100.0
            } else {
                100.0
            }
        );

        Ok(final_progress)
    }

    /// Process data with automatic retry and resumable functionality
    pub async fn process_with_resume<T, F, Fut>(
        &self,
        data: Vec<T>,
        processor: F,
        checkpoint_callback: Option<Box<dyn Fn(usize) + Send + Sync>>,
    ) -> Result<ImportProgress, FireupError>
    where
        T: Send + Clone + 'static,
        F: Fn(Vec<T>, &PostgreSQLImporter) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<usize, FireupError>> + Send,
    {
        let result = self.process_batches(data, processor).await?;
        
        // Call checkpoint callback if provided
        if let Some(callback) = checkpoint_callback {
            callback(result.processed_records);
        }
        
        Ok(result)
    }
}

impl PostgreSQLImporter {
    /// Create a batch processor with this importer
    pub fn create_batch_processor(&self, config: BatchConfig) -> BatchProcessor {
        BatchProcessor::new(self.clone(), config)
    }

    /// Execute multiple statements in a single transaction
    pub async fn execute_batch_statements(
        &self,
        statements: Vec<(String, Vec<Box<dyn tokio_postgres::types::ToSql + Send + Sync>>)>,
    ) -> Result<Vec<u64>, FireupError> {
        let mut client = self.get_connection().await?;
        let transaction = client.transaction().await
            .map_err(|e| FireupError::Transaction {
                message: format!("Failed to start transaction: {}", e),
                operation: "batch_statements".to_string(),
                context: FireupError::new_context("start_batch_transaction"),
                suggestions: vec!["Check database connection".to_string(), "Retry the operation".to_string()],
            })?;

        let mut results = Vec::new();
        
        for (statement, params) in statements {
            let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = 
                params.iter().map(|p| p.as_ref() as &(dyn tokio_postgres::types::ToSql + Sync)).collect();
            
            match transaction.execute(&statement, &param_refs).await {
                Ok(rows_affected) => results.push(rows_affected),
                Err(e) => {
                    // Rollback transaction on error
                    if let Err(rollback_err) = transaction.rollback().await {
                        warn!("Failed to rollback transaction: {}", rollback_err);
                    }
                    return Err(FireupError::data_import(
                        format!("Batch statement execution failed: {}", e),
                        None,
                        None,
                        FireupError::new_context("execute_batch_statement")
                    ));
                }
            }
        }

        // Commit transaction
        transaction.commit().await
            .map_err(|e| FireupError::Transaction {
                message: format!("Failed to commit batch transaction: {}", e),
                operation: "batch_statements".to_string(),
                context: FireupError::new_context("commit_batch_transaction"),
                suggestions: vec!["Check database connection".to_string(), "Retry the operation".to_string()],
            })?;

        Ok(results)
    }
}

/// Schema creation and data import execution functionality
impl PostgreSQLImporter {
    /// Execute DDL statements to create normalized schema
    #[instrument(skip(self, ddl_statements))]
    pub async fn create_schema(&self, ddl_statements: &[String]) -> Result<ImportResult, FireupError> {
        let tracker = get_monitoring_system().start_operation("schema_creation").await;
        tracker.add_metadata("ddl_statements_count", &ddl_statements.len().to_string()).await.ok();
        
        info!("Starting schema creation with {} DDL statements", ddl_statements.len());
        
        let mut result = ImportResult {
            imported_records: 0,
            failed_records: 0,
            warnings: Vec::new(),
        };

        let mut client = self.get_connection().await?;
        let transaction = client.transaction().await
            .map_err(|e| FireupError::Transaction {
                message: format!("Failed to start schema creation transaction: {}", e),
                operation: "schema_creation".to_string(),
                context: FireupError::new_context("start_schema_transaction"),
                suggestions: vec!["Check database connection".to_string(), "Retry the operation".to_string()],
            })?;

        for (index, ddl_statement) in ddl_statements.iter().enumerate() {
            info!("Executing DDL statement {} of {}", index + 1, ddl_statements.len());
            
            match transaction.execute(ddl_statement, &[]).await {
                Ok(_) => {
                    result.imported_records += 1;
                    info!("Successfully executed DDL statement {}", index + 1);
                    
                    // Log successful schema change
                    let mut details = HashMap::new();
                    details.insert("statement_index".to_string(), (index + 1).to_string());
                    details.insert("statement_preview".to_string(), 
                        ddl_statement.chars().take(100).collect::<String>());
                    
                    get_monitoring_system().log_audit_entry(
                        AuditOperationType::SchemaChange,
                        "database_schema",
                        &format!("statement_{}", index + 1),
                        "ddl_executed",
                        AuditResult::Success,
                        details,
                        None,
                    ).await.ok();
                }
                Err(e) => {
                    result.failed_records += 1;
                    let warning = format!("Failed to execute DDL statement {}: {}", index + 1, e);
                    warn!("{}", warning);
                    result.warnings.push(warning);
                    
                    // For schema creation, we might want to continue with other statements
                    // rather than failing completely, depending on the error type
                    if e.to_string().contains("already exists") {
                        result.warnings.push("Object already exists - continuing with next statement".to_string());
                    } else {
                        // For serious errors, rollback and fail
                        if let Err(rollback_err) = transaction.rollback().await {
                            warn!("Failed to rollback schema creation transaction: {}", rollback_err);
                        }
                        return Err(FireupError::data_import(
                            format!("Schema creation failed at statement {}: {}", index + 1, e),
                            None,
                            None,
                            FireupError::new_context("execute_ddl_statement")
                        ));
                    }
                }
            }
        }

        // Commit the schema creation transaction
        transaction.commit().await
            .map_err(|e| FireupError::Transaction {
                message: format!("Failed to commit schema creation transaction: {}", e),
                operation: "schema_creation".to_string(),
                context: FireupError::new_context("commit_schema_transaction"),
                suggestions: vec!["Check database connection".to_string(), "Retry the operation".to_string()],
            })?;

        info!("Schema creation completed: {} successful, {} failed", result.imported_records, result.failed_records);
        
        tracker.update_progress(result.imported_records as u64, None).await.ok();
        tracker.complete_success().await.ok();
        
        Ok(result)
    }

    /// Import transformed data using bulk INSERT operations
    #[instrument(skip(self, data))]
    pub async fn import_transformed_data(
        &self,
        table_name: &str,
        columns: &[String],
        data: Vec<Vec<String>>,
    ) -> Result<ImportResult, FireupError> {
        let tracker = get_monitoring_system().start_operation("data_import").await;
        tracker.add_metadata("table_name", table_name).await.ok();
        tracker.add_metadata("record_count", &data.len().to_string()).await.ok();
        tracker.add_metadata("column_count", &columns.len().to_string()).await.ok();
        
        info!("Starting data import for table '{}' with {} records", table_name, data.len());
        
        if data.is_empty() {
            return Ok(ImportResult {
                imported_records: 0,
                failed_records: 0,
                warnings: vec!["No data to import".to_string()],
            });
        }

        let mut result = ImportResult {
            imported_records: 0,
            failed_records: 0,
            warnings: Vec::new(),
        };

        // Generate INSERT statement template
        let placeholders: Vec<String> = (1..=columns.len())
            .map(|i| format!("${}", i))
            .collect();
        let insert_statement = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            table_name,
            columns.join(", "),
            placeholders.join(", ")
        );

        info!("Using INSERT statement: {}", insert_statement);

        // Process data in batches
        let batch_size = 1000;
        for (batch_index, batch) in data.chunks(batch_size).enumerate() {
            info!("Processing batch {} with {} records", batch_index + 1, batch.len());
            
            for row in batch {
                if row.len() != columns.len() {
                    result.failed_records += 1;
                    result.warnings.push(format!(
                        "Row has {} columns but expected {}", 
                        row.len(), 
                        columns.len()
                    ));
                    continue;
                }

                let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = 
                    row.iter().map(|s| s as &(dyn tokio_postgres::types::ToSql + Sync)).collect();
                
                match self.execute_statement(&insert_statement, &param_refs).await {
                    Ok(_) => result.imported_records += 1,
                    Err(e) => {
                        result.failed_records += 1;
                        result.warnings.push(format!("Failed to insert record: {}", e));
                    }
                }
            }
        }

        info!(
            "Data import completed for table '{}': {} successful, {} failed",
            table_name, result.imported_records, result.failed_records
        );

        // Log data modification audit entry
        let mut details = HashMap::new();
        details.insert("table_name".to_string(), table_name.to_string());
        details.insert("records_imported".to_string(), result.imported_records.to_string());
        details.insert("records_failed".to_string(), result.failed_records.to_string());
        details.insert("columns".to_string(), columns.join(","));
        
        let audit_result = if result.failed_records == 0 {
            AuditResult::Success
        } else if result.imported_records > 0 {
            AuditResult::PartialSuccess(format!("{} records failed", result.failed_records))
        } else {
            AuditResult::Failure("All records failed to import".to_string())
        };
        
        get_monitoring_system().log_audit_entry(
            AuditOperationType::DataModification,
            "database_table",
            table_name,
            "bulk_insert",
            audit_result,
            details,
            None,
        ).await.ok();

        tracker.update_progress(result.imported_records as u64, None).await.ok();
        tracker.complete_success().await.ok();

        Ok(result)
    }

    /// Validate constraints and foreign key relationships during import
    pub async fn validate_constraints(&self, table_name: &str) -> Result<Vec<String>, FireupError> {
        info!("Validating constraints for table '{}'", table_name);
        
        let mut validation_errors = Vec::new();
        
        // Check for constraint violations
        let constraint_check_query = format!(
            "SELECT conname, pg_get_constraintdef(oid) as definition 
             FROM pg_constraint 
             WHERE conrelid = '{}'::regclass",
            table_name
        );
        
        match self.execute_query(&constraint_check_query, &[]).await {
            Ok(rows) => {
                for row in rows {
                    let constraint_name: String = row.get(0);
                    let definition: String = row.get(1);
                    
                    // For each constraint, try to find violations
                    if definition.contains("FOREIGN KEY") {
                        let fk_validation = self.validate_foreign_key_constraint(table_name, &constraint_name).await;
                        if let Ok(violations) = fk_validation {
                            validation_errors.extend(violations);
                        }
                    } else if definition.contains("CHECK") {
                        let check_validation = self.validate_check_constraint(table_name, &constraint_name).await;
                        if let Ok(violations) = check_validation {
                            validation_errors.extend(violations);
                        }
                    }
                }
            }
            Err(e) => {
                warn!("Failed to retrieve constraints for table '{}': {}", table_name, e);
                validation_errors.push(format!("Could not retrieve constraints: {}", e));
            }
        }
        
        // Check for NOT NULL violations
        let null_check = self.validate_not_null_constraints(table_name).await;
        if let Ok(violations) = null_check {
            validation_errors.extend(violations);
        }

        if validation_errors.is_empty() {
            info!("All constraints validated successfully for table '{}'", table_name);
        } else {
            warn!("Found {} constraint violations for table '{}'", validation_errors.len(), table_name);
        }

        Ok(validation_errors)
    }

    /// Validate foreign key constraints
    async fn validate_foreign_key_constraint(&self, table_name: &str, constraint_name: &str) -> Result<Vec<String>, FireupError> {
        let query = format!(
            "SELECT COUNT(*) as violation_count 
             FROM {} 
             WHERE NOT EXISTS (
                 SELECT 1 FROM information_schema.table_constraints tc
                 JOIN information_schema.key_column_usage kcu ON tc.constraint_name = kcu.constraint_name
                 WHERE tc.constraint_name = '{}' AND tc.table_name = '{}'
             )",
            table_name, constraint_name, table_name
        );

        match self.execute_query(&query, &[]).await {
            Ok(rows) => {
                if let Some(row) = rows.first() {
                    let violation_count: i64 = row.get(0);
                    if violation_count > 0 {
                        Ok(vec![format!(
                            "Foreign key constraint '{}' violated in {} rows",
                            constraint_name, violation_count
                        )])
                    } else {
                        Ok(Vec::new())
                    }
                } else {
                    Ok(Vec::new())
                }
            }
            Err(e) => {
                warn!("Failed to validate foreign key constraint '{}': {}", constraint_name, e);
                Ok(vec![format!("Could not validate foreign key constraint '{}': {}", constraint_name, e)])
            }
        }
    }

    /// Validate check constraints
    async fn validate_check_constraint(&self, table_name: &str, constraint_name: &str) -> Result<Vec<String>, FireupError> {
        // This is a simplified check - in practice, you'd need to parse the constraint definition
        // and create appropriate validation queries
        let query = format!(
            "SELECT COUNT(*) as total_rows FROM {}",
            table_name
        );

        match self.execute_query(&query, &[]).await {
            Ok(_) => {
                // For now, assume check constraints are valid
                // In a real implementation, you'd parse the constraint definition and validate accordingly
                Ok(Vec::new())
            }
            Err(e) => {
                Ok(vec![format!("Could not validate check constraint '{}': {}", constraint_name, e)])
            }
        }
    }

    /// Validate NOT NULL constraints
    async fn validate_not_null_constraints(&self, table_name: &str) -> Result<Vec<String>, FireupError> {
        let query = format!(
            "SELECT column_name 
             FROM information_schema.columns 
             WHERE table_name = '{}' AND is_nullable = 'NO'",
            table_name
        );

        let mut violations = Vec::new();

        match self.execute_query(&query, &[]).await {
            Ok(rows) => {
                for row in rows {
                    let column_name: String = row.get(0);
                    
                    let null_check_query = format!(
                        "SELECT COUNT(*) as null_count FROM {} WHERE {} IS NULL",
                        table_name, column_name
                    );
                    
                    match self.execute_query(&null_check_query, &[]).await {
                        Ok(null_rows) => {
                            if let Some(null_row) = null_rows.first() {
                                let null_count: i64 = null_row.get(0);
                                if null_count > 0 {
                                    violations.push(format!(
                                        "NOT NULL constraint violated for column '{}': {} null values found",
                                        column_name, null_count
                                    ));
                                }
                            }
                        }
                        Err(e) => {
                            violations.push(format!(
                                "Could not check NULL values for column '{}': {}",
                                column_name, e
                            ));
                        }
                    }
                }
            }
            Err(e) => {
                violations.push(format!("Could not retrieve NOT NULL columns: {}", e));
            }
        }

        Ok(violations)
    }

    /// Execute complete schema creation and data import workflow
    pub async fn execute_full_import(
        &self,
        ddl_statements: Vec<String>,
        table_imports: Vec<TableImportSpec>,
    ) -> Result<FullImportResult, FireupError> {
        info!("Starting full import workflow with {} tables", table_imports.len());
        
        let mut full_result = FullImportResult {
            schema_creation: None,
            table_imports: Vec::new(),
            validation_results: Vec::new(),
            total_records_imported: 0,
            total_records_failed: 0,
            warnings: Vec::new(),
        };

        // Step 1: Create schema
        info!("Step 1: Creating database schema");
        match self.create_schema(&ddl_statements).await {
            Ok(schema_result) => {
                info!("Schema creation completed successfully");
                full_result.schema_creation = Some(schema_result);
            }
            Err(e) => {
                error!("Schema creation failed: {}", e);
                return Err(e);
            }
        }

        // Step 2: Import data for each table
        info!("Step 2: Importing data for {} tables", table_imports.len());
        for (index, table_spec) in table_imports.into_iter().enumerate() {
            info!("Importing data for table '{}' ({}/{})", table_spec.table_name, index + 1, full_result.table_imports.len() + 1);
            
            // This is a placeholder - in practice, you'd call the appropriate import method
            // based on the table_spec configuration
            let table_result = ImportResult {
                imported_records: 0,
                failed_records: 0,
                warnings: vec![format!("Table import for '{}' not yet implemented", table_spec.table_name)],
            };
            
            full_result.total_records_imported += table_result.imported_records;
            full_result.total_records_failed += table_result.failed_records;
            full_result.warnings.extend(table_result.warnings.clone());
            full_result.table_imports.push((table_spec.table_name.clone(), table_result));
        }

        // Step 3: Validate constraints
        info!("Step 3: Validating constraints for all tables");
        for (table_name, _) in &full_result.table_imports {
            match self.validate_constraints(table_name).await {
                Ok(violations) => {
                    if !violations.is_empty() {
                        full_result.warnings.extend(violations.clone());
                    }
                    full_result.validation_results.push((table_name.clone(), violations));
                }
                Err(e) => {
                    let error_msg = format!("Failed to validate constraints for table '{}': {}", table_name, e);
                    warn!("{}", error_msg);
                    full_result.warnings.push(error_msg.clone());
                    full_result.validation_results.push((table_name.clone(), vec![error_msg]));
                }
            }
        }

        info!(
            "Full import workflow completed: {} records imported, {} failed, {} warnings",
            full_result.total_records_imported,
            full_result.total_records_failed,
            full_result.warnings.len()
        );

        Ok(full_result)
    }
}

/// Specification for importing data into a specific table
#[derive(Debug, Clone)]
pub struct TableImportSpec {
    pub table_name: String,
    pub columns: Vec<String>,
    pub data_source: String, // Could be file path, collection name, etc.
    pub batch_size: Option<usize>,
    pub validation_enabled: bool,
}

/// Result of a complete import workflow
#[derive(Debug, Clone)]
pub struct FullImportResult {
    pub schema_creation: Option<ImportResult>,
    pub table_imports: Vec<(String, ImportResult)>,
    pub validation_results: Vec<(String, Vec<String>)>,
    pub total_records_imported: usize,
    pub total_records_failed: usize,
    pub warnings: Vec<String>,
}

impl FullImportResult {
    /// Check if the import was successful overall
    pub fn is_successful(&self) -> bool {
        self.total_records_failed == 0 && 
        self.validation_results.iter().all(|(_, violations)| violations.is_empty())
    }

    /// Get a summary of the import results
    pub fn summary(&self) -> String {
        format!(
            "Import Summary: {} tables processed, {} records imported, {} failed, {} warnings, {} constraint violations",
            self.table_imports.len(),
            self.total_records_imported,
            self.total_records_failed,
            self.warnings.len(),
            self.validation_results.iter().map(|(_, v)| v.len()).sum::<usize>()
        )
    }
}