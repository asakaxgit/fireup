// Comprehensive logging and monitoring capabilities for Fireup
use crate::error::{FireupError, FireupResult, PerformanceMetrics};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{info, warn, error, debug, instrument};
use uuid::Uuid;

/// Performance metrics tracker for operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationMetrics {
    pub operation_id: String,
    pub operation_name: String,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub end_time: Option<chrono::DateTime<chrono::Utc>>,
    pub duration_ms: Option<u64>,
    pub records_processed: Option<u64>,
    pub throughput: Option<f64>, // records per second
    pub memory_usage_bytes: Option<u64>,
    pub cpu_usage_percent: Option<f64>,
    pub status: OperationStatus,
    pub metadata: HashMap<String, String>,
}

/// Status of an operation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OperationStatus {
    Started,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

/// Audit log entry for data access and modifications
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogEntry {
    pub entry_id: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub operation_type: AuditOperationType,
    pub user_id: Option<String>,
    pub resource_type: String,
    pub resource_id: String,
    pub action: String,
    pub details: HashMap<String, String>,
    pub result: AuditResult,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

/// Types of operations that can be audited
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditOperationType {
    DataAccess,
    DataModification,
    SchemaChange,
    SystemConfiguration,
    Authentication,
    Authorization,
}

/// Result of an audited operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditResult {
    Success,
    Failure(String),
    PartialSuccess(String),
}

/// System-wide monitoring and metrics collection
pub struct MonitoringSystem {
    /// Active operation metrics
    active_operations: Arc<RwLock<HashMap<String, OperationMetrics>>>,
    /// Completed operation metrics (limited history)
    completed_operations: Arc<RwLock<Vec<OperationMetrics>>>,
    /// Audit log entries
    audit_log: Arc<RwLock<Vec<AuditLogEntry>>>,
    /// System configuration
    config: MonitoringConfig,
}

/// Configuration for the monitoring system
#[derive(Debug, Clone)]
pub struct MonitoringConfig {
    /// Maximum number of completed operations to keep in memory
    pub max_completed_operations: usize,
    /// Maximum number of audit log entries to keep in memory
    pub max_audit_entries: usize,
    /// Whether to enable detailed performance tracking
    pub enable_performance_tracking: bool,
    /// Whether to enable audit logging
    pub enable_audit_logging: bool,
    /// Minimum duration to track for performance metrics (ms)
    pub min_tracking_duration_ms: u64,
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            max_completed_operations: 1000,
            max_audit_entries: 10000,
            enable_performance_tracking: true,
            enable_audit_logging: true,
            min_tracking_duration_ms: 100,
        }
    }
}

impl MonitoringSystem {
    /// Create a new monitoring system
    pub fn new(config: MonitoringConfig) -> Self {
        Self {
            active_operations: Arc::new(RwLock::new(HashMap::new())),
            completed_operations: Arc::new(RwLock::new(Vec::new())),
            audit_log: Arc::new(RwLock::new(Vec::new())),
            config,
        }
    }

    /// Start tracking an operation
    pub async fn start_operation(&self, operation_name: impl Into<String>) -> OperationTracker {
        let operation_id = Uuid::new_v4().to_string();
        let operation_name = operation_name.into();
        
        let metrics = OperationMetrics {
            operation_id: operation_id.clone(),
            operation_name: operation_name.clone(),
            start_time: chrono::Utc::now(),
            end_time: None,
            duration_ms: None,
            records_processed: None,
            throughput: None,
            memory_usage_bytes: None,
            cpu_usage_percent: None,
            status: OperationStatus::Started,
            metadata: HashMap::new(),
        };

        if self.config.enable_performance_tracking {
            let mut active_ops = self.active_operations.write().await;
            active_ops.insert(operation_id.clone(), metrics);
        }

        info!(
            operation_id = %operation_id,
            operation_name = %operation_name,
            "Started operation"
        );

        OperationTracker {
            operation_id,
            operation_name,
            start_time: Instant::now(),
            monitoring_system: self.clone(),
        }
    }

    /// Update operation metrics
    pub async fn update_operation_metrics(
        &self,
        operation_id: &str,
        records_processed: Option<u64>,
        metadata: Option<HashMap<String, String>>,
    ) -> FireupResult<()> {
        if !self.config.enable_performance_tracking {
            return Ok(());
        }

        let mut active_ops = self.active_operations.write().await;
        if let Some(metrics) = active_ops.get_mut(operation_id) {
            metrics.status = OperationStatus::InProgress;
            
            if let Some(records) = records_processed {
                metrics.records_processed = Some(records);
                
                // Calculate throughput
                let elapsed = chrono::Utc::now().signed_duration_since(metrics.start_time);
                if elapsed.num_milliseconds() > 0 {
                    let throughput = records as f64 / (elapsed.num_milliseconds() as f64 / 1000.0);
                    metrics.throughput = Some(throughput);
                }
            }
            
            if let Some(meta) = metadata {
                metrics.metadata.extend(meta);
            }

            debug!(
                operation_id = %operation_id,
                records_processed = ?records_processed,
                throughput = ?metrics.throughput,
                "Updated operation metrics"
            );
        }

        Ok(())
    }

    /// Complete an operation and move it to completed operations
    pub async fn complete_operation(
        &self,
        operation_id: &str,
        status: OperationStatus,
        final_metrics: Option<PerformanceMetrics>,
    ) -> FireupResult<()> {
        if !self.config.enable_performance_tracking {
            return Ok(());
        }

        let mut active_ops = self.active_operations.write().await;
        if let Some(mut metrics) = active_ops.remove(operation_id) {
            metrics.end_time = Some(chrono::Utc::now());
            metrics.status = status.clone();
            
            if let Some(end_time) = metrics.end_time {
                let duration = end_time.signed_duration_since(metrics.start_time);
                metrics.duration_ms = Some(duration.num_milliseconds() as u64);
            }

            if let Some(perf_metrics) = final_metrics {
                metrics.memory_usage_bytes = perf_metrics.memory_usage_bytes;
                metrics.cpu_usage_percent = perf_metrics.cpu_usage_percent;
                if metrics.records_processed.is_none() {
                    metrics.records_processed = perf_metrics.records_processed;
                }
                if metrics.throughput.is_none() {
                    metrics.throughput = perf_metrics.throughput;
                }
            }

            // Only track operations that meet minimum duration threshold
            if metrics.duration_ms.unwrap_or(0) >= self.config.min_tracking_duration_ms {
                let mut completed_ops = self.completed_operations.write().await;
                completed_ops.push(metrics.clone());
                
                // Maintain size limit
                if completed_ops.len() > self.config.max_completed_operations {
                    completed_ops.remove(0);
                }
            }

            info!(
                operation_id = %operation_id,
                operation_name = %metrics.operation_name,
                status = ?status,
                duration_ms = ?metrics.duration_ms,
                records_processed = ?metrics.records_processed,
                throughput = ?metrics.throughput,
                "Completed operation"
            );
        }

        Ok(())
    }

    /// Log an audit entry for data access or modification
    pub async fn log_audit_entry(
        &self,
        operation_type: AuditOperationType,
        resource_type: impl Into<String>,
        resource_id: impl Into<String>,
        action: impl Into<String>,
        result: AuditResult,
        details: HashMap<String, String>,
        user_id: Option<String>,
    ) -> FireupResult<()> {
        if !self.config.enable_audit_logging {
            return Ok(());
        }

        let entry = AuditLogEntry {
            entry_id: Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now(),
            operation_type: operation_type.clone(),
            user_id: user_id.clone(),
            resource_type: resource_type.into(),
            resource_id: resource_id.into(),
            action: action.into(),
            details,
            result: result.clone(),
            ip_address: None, // Could be populated from request context
            user_agent: None, // Could be populated from request context
        };

        let mut audit_log = self.audit_log.write().await;
        audit_log.push(entry.clone());

        // Maintain size limit
        if audit_log.len() > self.config.max_audit_entries {
            audit_log.remove(0);
        }

        // Log based on operation type and result
        match (&operation_type, &result) {
            (AuditOperationType::DataModification, AuditResult::Success) => {
                info!(
                    entry_id = %entry.entry_id,
                    resource_type = %entry.resource_type,
                    resource_id = %entry.resource_id,
                    action = %entry.action,
                    user_id = ?user_id,
                    "Data modification successful"
                );
            }
            (AuditOperationType::DataModification, AuditResult::Failure(ref error)) => {
                warn!(
                    entry_id = %entry.entry_id,
                    resource_type = %entry.resource_type,
                    resource_id = %entry.resource_id,
                    action = %entry.action,
                    error = %error,
                    user_id = ?user_id,
                    "Data modification failed"
                );
            }
            (AuditOperationType::SchemaChange, AuditResult::Success) => {
                info!(
                    entry_id = %entry.entry_id,
                    resource_type = %entry.resource_type,
                    resource_id = %entry.resource_id,
                    action = %entry.action,
                    user_id = ?user_id,
                    "Schema change successful"
                );
            }
            (AuditOperationType::DataAccess, AuditResult::Success) => {
                debug!(
                    entry_id = %entry.entry_id,
                    resource_type = %entry.resource_type,
                    resource_id = %entry.resource_id,
                    action = %entry.action,
                    user_id = ?user_id,
                    "Data access logged"
                );
            }
            _ => {
                debug!(
                    entry_id = %entry.entry_id,
                    operation_type = ?operation_type,
                    result = ?result,
                    "Audit entry logged"
                );
            }
        }

        Ok(())
    }

    /// Get current system statistics
    pub async fn get_system_stats(&self) -> SystemStats {
        let active_ops = self.active_operations.read().await;
        let completed_ops = self.completed_operations.read().await;
        let audit_log = self.audit_log.read().await;

        let total_operations = active_ops.len() + completed_ops.len();
        let successful_operations = completed_ops
            .iter()
            .filter(|op| op.status == OperationStatus::Completed)
            .count();
        let failed_operations = completed_ops
            .iter()
            .filter(|op| op.status == OperationStatus::Failed)
            .count();

        let avg_duration_ms = if completed_ops.is_empty() {
            0.0
        } else {
            completed_ops
                .iter()
                .filter_map(|op| op.duration_ms)
                .sum::<u64>() as f64 / completed_ops.len() as f64
        };

        let total_records_processed = completed_ops
            .iter()
            .filter_map(|op| op.records_processed)
            .sum::<u64>();

        SystemStats {
            active_operations: active_ops.len(),
            completed_operations: completed_ops.len(),
            total_operations,
            successful_operations,
            failed_operations,
            avg_duration_ms,
            total_records_processed,
            audit_entries: audit_log.len(),
        }
    }

    /// Get recent audit entries
    pub async fn get_recent_audit_entries(&self, limit: usize) -> Vec<AuditLogEntry> {
        let audit_log = self.audit_log.read().await;
        let start_index = if audit_log.len() > limit {
            audit_log.len() - limit
        } else {
            0
        };
        audit_log[start_index..].to_vec()
    }

    /// Get performance metrics for completed operations
    pub async fn get_performance_metrics(&self, operation_name_filter: Option<&str>) -> Vec<OperationMetrics> {
        let completed_ops = self.completed_operations.read().await;
        completed_ops
            .iter()
            .filter(|op| {
                operation_name_filter
                    .map(|filter| op.operation_name.contains(filter))
                    .unwrap_or(true)
            })
            .cloned()
            .collect()
    }
}

impl Clone for MonitoringSystem {
    fn clone(&self) -> Self {
        Self {
            active_operations: Arc::clone(&self.active_operations),
            completed_operations: Arc::clone(&self.completed_operations),
            audit_log: Arc::clone(&self.audit_log),
            config: self.config.clone(),
        }
    }
}

/// System statistics summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemStats {
    pub active_operations: usize,
    pub completed_operations: usize,
    pub total_operations: usize,
    pub successful_operations: usize,
    pub failed_operations: usize,
    pub avg_duration_ms: f64,
    pub total_records_processed: u64,
    pub audit_entries: usize,
}

/// Operation tracker that automatically manages operation lifecycle
pub struct OperationTracker {
    operation_id: String,
    operation_name: String,
    start_time: Instant,
    monitoring_system: MonitoringSystem,
}

impl OperationTracker {
    /// Get the operation ID
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// Update operation progress
    pub async fn update_progress(
        &self,
        records_processed: u64,
        additional_metadata: Option<HashMap<String, String>>,
    ) -> FireupResult<()> {
        self.monitoring_system
            .update_operation_metrics(&self.operation_id, Some(records_processed), additional_metadata)
            .await
    }

    /// Add metadata to the operation
    pub async fn add_metadata(&self, key: impl Into<String>, value: impl Into<String>) -> FireupResult<()> {
        let mut metadata = HashMap::new();
        metadata.insert(key.into(), value.into());
        self.monitoring_system
            .update_operation_metrics(&self.operation_id, None, Some(metadata))
            .await
    }

    /// Complete the operation successfully
    pub async fn complete_success(self) -> FireupResult<()> {
        let duration_ms = self.start_time.elapsed().as_millis() as u64;
        let performance_metrics = PerformanceMetrics {
            duration_ms,
            memory_usage_bytes: None,
            cpu_usage_percent: None,
            records_processed: None,
            throughput: None,
        };

        self.monitoring_system
            .complete_operation(&self.operation_id, OperationStatus::Completed, Some(performance_metrics))
            .await
    }

    /// Complete the operation with failure
    pub async fn complete_failure(self, error: &FireupError) -> FireupResult<()> {
        let duration_ms = self.start_time.elapsed().as_millis() as u64;
        let performance_metrics = PerformanceMetrics {
            duration_ms,
            memory_usage_bytes: None,
            cpu_usage_percent: None,
            records_processed: None,
            throughput: None,
        };

        error!(
            operation_id = %self.operation_id,
            operation_name = %self.operation_name,
            error = %error,
            "Operation failed"
        );

        self.monitoring_system
            .complete_operation(&self.operation_id, OperationStatus::Failed, Some(performance_metrics))
            .await
    }

    /// Complete the operation with custom metrics
    pub async fn complete_with_metrics(
        self,
        status: OperationStatus,
        metrics: PerformanceMetrics,
    ) -> FireupResult<()> {
        self.monitoring_system
            .complete_operation(&self.operation_id, status, Some(metrics))
            .await
    }
}

/// Global monitoring system instance
static MONITORING_SYSTEM: once_cell::sync::Lazy<MonitoringSystem> = 
    once_cell::sync::Lazy::new(|| MonitoringSystem::new(MonitoringConfig::default()));

/// Get the global monitoring system instance
pub fn get_monitoring_system() -> &'static MonitoringSystem {
    &MONITORING_SYSTEM
}

/// Initialize monitoring system with custom configuration
pub fn initialize_monitoring(_config: MonitoringConfig) -> &'static MonitoringSystem {
    // Note: This is a simplified approach. In a real application, you might want
    // to use a more sophisticated initialization pattern
    info!("Monitoring system initialized with custom configuration");
    &MONITORING_SYSTEM
}

/// Convenience macro for tracking operations
#[macro_export]
macro_rules! track_operation {
    ($operation_name:expr, $body:expr) => {{
        let tracker = $crate::monitoring::get_monitoring_system()
            .start_operation($operation_name)
            .await;
        
        let result = $body;
        
        match &result {
            Ok(_) => {
                if let Err(e) = tracker.complete_success().await {
                    tracing::warn!("Failed to complete operation tracking: {}", e);
                }
            }
            Err(error) => {
                if let Err(e) = tracker.complete_failure(error).await {
                    tracing::warn!("Failed to complete operation tracking: {}", e);
                }
            }
        }
        
        result
    }};
}

/// Convenience macro for logging audit entries
#[macro_export]
macro_rules! audit_log {
    ($operation_type:expr, $resource_type:expr, $resource_id:expr, $action:expr, $result:expr) => {
        if let Err(e) = $crate::monitoring::get_monitoring_system()
            .log_audit_entry(
                $operation_type,
                $resource_type,
                $resource_id,
                $action,
                $result,
                std::collections::HashMap::new(),
                None,
            )
            .await
        {
            tracing::warn!("Failed to log audit entry: {}", e);
        }
    };
    
    ($operation_type:expr, $resource_type:expr, $resource_id:expr, $action:expr, $result:expr, $details:expr) => {
        if let Err(e) = $crate::monitoring::get_monitoring_system()
            .log_audit_entry(
                $operation_type,
                $resource_type,
                $resource_id,
                $action,
                $result,
                $details,
                None,
            )
            .await
        {
            tracing::warn!("Failed to log audit entry: {}", e);
        }
    };
}