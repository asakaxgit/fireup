// Comprehensive monitoring and audit logging system
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use chrono::{DateTime, Utc};
use serde::{Serialize, Deserialize};
use tracing::{info, warn, error, debug};
use uuid::Uuid;

/// Configuration for the monitoring system
#[derive(Debug, Clone)]
pub struct MonitoringConfig {
    pub enable_performance_tracking: bool,
    pub enable_audit_logging: bool,
    pub max_completed_operations: usize,
    pub max_audit_entries: usize,
    pub min_tracking_duration_ms: u64,
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            enable_performance_tracking: true,
            enable_audit_logging: true,
            max_completed_operations: 1000,
            max_audit_entries: 10000,
            min_tracking_duration_ms: 100,
        }
    }
}

/// Types of audit operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditOperationType {
    DataAccess,
    DataModification,
    SchemaChange,
    SystemConfiguration,
}

/// Results of audit operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditResult {
    Success,
    PartialSuccess(String),
    Failure(String),
}

/// Status of an operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OperationStatus {
    InProgress,
    Completed,
    Failed,
}

/// Performance metrics for an operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetric {
    pub operation_id: String,
    pub operation_name: String,
    pub status: OperationStatus,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub records_processed: Option<u64>,
    pub throughput: Option<f64>, // records per second
    pub metadata: HashMap<String, String>,
}

/// Audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub operation_type: AuditOperationType,
    pub resource_type: String,
    pub resource_id: String,
    pub action: String,
    pub result: AuditResult,
    pub details: HashMap<String, String>,
    pub user_id: Option<String>,
}

/// System statistics
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

/// Operation tracker for monitoring individual operations
pub struct OperationTracker {
    pub operation_id: String,
    pub operation_name: String,
    pub start_time: DateTime<Utc>,
    pub metadata: Arc<Mutex<HashMap<String, String>>>,
    pub monitoring_system: Arc<MonitoringSystem>,
}

impl OperationTracker {
    /// Mark operation as successfully completed
    pub async fn complete_success(&self) -> Result<(), String> {
        let duration = Utc::now().signed_duration_since(self.start_time);
        let duration_ms = duration.num_milliseconds() as u64;
        
        info!(
            operation_id = %self.operation_id,
            operation_name = %self.operation_name,
            duration_ms = duration_ms,
            "Operation completed successfully"
        );
        
        self.monitoring_system.complete_operation(
            &self.operation_id,
            OperationStatus::Completed,
            Some(duration_ms),
        ).await;
        
        Ok(())
    }
    
    /// Mark operation as failed
    pub async fn complete_failure(&self, error: &str) -> Result<(), String> {
        let duration = Utc::now().signed_duration_since(self.start_time);
        let duration_ms = duration.num_milliseconds() as u64;
        
        error!(
            operation_id = %self.operation_id,
            operation_name = %self.operation_name,
            duration_ms = duration_ms,
            error = error,
            "Operation failed"
        );
        
        self.monitoring_system.complete_operation(
            &self.operation_id,
            OperationStatus::Failed,
            Some(duration_ms),
        ).await;
        
        Ok(())
    }
    
    /// Add metadata to the operation
    pub async fn add_metadata(&self, key: &str, value: &str) -> Result<(), String> {
        if let Ok(mut metadata) = self.metadata.lock() {
            metadata.insert(key.to_string(), value.to_string());
            debug!(
                operation_id = %self.operation_id,
                key = key,
                value = value,
                "Added operation metadata"
            );
        }
        Ok(())
    }
    
    /// Update operation progress
    pub async fn update_progress(&self, processed: u64, total: Option<u64>) -> Result<(), String> {
        let progress_info = if let Some(total) = total {
            format!("{}/{} ({:.1}%)", processed, total, (processed as f64 / total as f64) * 100.0)
        } else {
            format!("{} processed", processed)
        };
        
        debug!(
            operation_id = %self.operation_id,
            processed = processed,
            total = ?total,
            "Operation progress updated: {}", progress_info
        );
        
        self.monitoring_system.update_operation_progress(
            &self.operation_id,
            processed,
            total,
        ).await;
        
        Ok(())
    }
}

/// Main monitoring system
pub struct MonitoringSystem {
    config: MonitoringConfig,
    active_operations: Arc<Mutex<HashMap<String, PerformanceMetric>>>,
    completed_operations: Arc<Mutex<Vec<PerformanceMetric>>>,
    audit_log: Arc<Mutex<Vec<AuditEntry>>>,
}

impl MonitoringSystem {
    /// Create a new monitoring system with configuration
    pub fn new(config: MonitoringConfig) -> Self {
        info!("Initializing monitoring system with config: {:?}", config);
        
        Self {
            config,
            active_operations: Arc::new(Mutex::new(HashMap::new())),
            completed_operations: Arc::new(Mutex::new(Vec::new())),
            audit_log: Arc::new(Mutex::new(Vec::new())),
        }
    }
    
    /// Start tracking a new operation
    pub async fn start_operation(&self, operation_name: &str) -> OperationTracker {
        let operation_id = Uuid::new_v4().to_string();
        let start_time = Utc::now();
        
        if self.config.enable_performance_tracking {
            let metric = PerformanceMetric {
                operation_id: operation_id.clone(),
                operation_name: operation_name.to_string(),
                status: OperationStatus::InProgress,
                start_time,
                end_time: None,
                duration_ms: None,
                records_processed: None,
                throughput: None,
                metadata: HashMap::new(),
            };
            
            if let Ok(mut active) = self.active_operations.lock() {
                active.insert(operation_id.clone(), metric);
            }
        }
        
        info!(
            operation_id = %operation_id,
            operation_name = operation_name,
            "Started operation tracking"
        );
        
        OperationTracker {
            operation_id,
            operation_name: operation_name.to_string(),
            start_time,
            metadata: Arc::new(Mutex::new(HashMap::new())),
            monitoring_system: Arc::new(MonitoringSystem::new(self.config.clone())),
        }
    }
    
    /// Complete an operation
    async fn complete_operation(
        &self,
        operation_id: &str,
        status: OperationStatus,
        duration_ms: Option<u64>,
    ) {
        if !self.config.enable_performance_tracking {
            return;
        }
        
        let mut metric = None;
        
        // Remove from active operations
        if let Ok(mut active) = self.active_operations.lock() {
            metric = active.remove(operation_id);
        }
        
        if let Some(mut metric) = metric {
            metric.status = status;
            metric.end_time = Some(Utc::now());
            metric.duration_ms = duration_ms;
            
            // Calculate throughput if records were processed
            if let (Some(duration), Some(records)) = (duration_ms, metric.records_processed) {
                if duration > 0 {
                    metric.throughput = Some((records as f64) / (duration as f64 / 1000.0));
                }
            }
            
            // Add to completed operations (with size limit)
            if let Ok(mut completed) = self.completed_operations.lock() {
                completed.push(metric);
                
                // Maintain size limit
                if completed.len() > self.config.max_completed_operations {
                    completed.remove(0);
                }
            }
        }
    }
    
    /// Update operation progress
    async fn update_operation_progress(
        &self,
        operation_id: &str,
        processed: u64,
        _total: Option<u64>,
    ) {
        if !self.config.enable_performance_tracking {
            return;
        }
        
        if let Ok(mut active) = self.active_operations.lock() {
            if let Some(metric) = active.get_mut(operation_id) {
                metric.records_processed = Some(processed);
            }
        }
    }
    
    /// Log an audit entry
    pub async fn log_audit_entry(
        &self,
        operation_type: AuditOperationType,
        resource_type: &str,
        resource_id: &str,
        action: &str,
        result: AuditResult,
        details: HashMap<String, String>,
        user_id: Option<String>,
    ) -> Result<(), String> {
        if !self.config.enable_audit_logging {
            return Ok(());
        }
        
        let entry = AuditEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            operation_type: operation_type.clone(),
            resource_type: resource_type.to_string(),
            resource_id: resource_id.to_string(),
            action: action.to_string(),
            result: result.clone(),
            details,
            user_id,
        };
        
        // Log the audit entry
        match &result {
            AuditResult::Success => {
                info!(
                    audit_id = %entry.id,
                    operation_type = ?operation_type,
                    resource_type = resource_type,
                    resource_id = resource_id,
                    action = action,
                    "Audit: {} {} on {} - SUCCESS", action, resource_type, resource_id
                );
            }
            AuditResult::PartialSuccess(msg) => {
                warn!(
                    audit_id = %entry.id,
                    operation_type = ?operation_type,
                    resource_type = resource_type,
                    resource_id = resource_id,
                    action = action,
                    message = msg,
                    "Audit: {} {} on {} - PARTIAL SUCCESS: {}", action, resource_type, resource_id, msg
                );
            }
            AuditResult::Failure(msg) => {
                error!(
                    audit_id = %entry.id,
                    operation_type = ?operation_type,
                    resource_type = resource_type,
                    resource_id = resource_id,
                    action = action,
                    error = msg,
                    "Audit: {} {} on {} - FAILURE: {}", action, resource_type, resource_id, msg
                );
            }
        }
        
        // Store in audit log (with size limit)
        if let Ok(mut audit_log) = self.audit_log.lock() {
            audit_log.push(entry);
            
            // Maintain size limit
            if audit_log.len() > self.config.max_audit_entries {
                audit_log.remove(0);
            }
        }
        
        Ok(())
    }
    
    /// Get system statistics
    pub async fn get_system_stats(&self) -> SystemStats {
        let (active_count, completed_ops, total_records) = {
            let active_result = self.active_operations.lock();
            let completed_result = self.completed_operations.lock();
            
            match (active_result, completed_result) {
                (Ok(active), Ok(completed)) => {
                    let total_records = completed.iter()
                        .filter_map(|op| op.records_processed)
                        .sum::<u64>();
                    
                    (active.len(), completed.clone(), total_records)
                }
                _ => {
                    warn!("Failed to lock operations for stats");
                    (0, Vec::new(), 0)
                }
            }
        };
        
        let audit_count = self.audit_log.lock()
            .map(|log| log.len())
            .unwrap_or(0);
        
        let total_operations = active_count + completed_ops.len();
        let successful_operations = completed_ops.iter()
            .filter(|op| matches!(op.status, OperationStatus::Completed))
            .count();
        let failed_operations = completed_ops.iter()
            .filter(|op| matches!(op.status, OperationStatus::Failed))
            .count();
        
        let avg_duration_ms = if !completed_ops.is_empty() {
            completed_ops.iter()
                .filter_map(|op| op.duration_ms)
                .map(|d| d as f64)
                .sum::<f64>() / completed_ops.len() as f64
        } else {
            0.0
        };
        
        SystemStats {
            active_operations: active_count,
            completed_operations: completed_ops.len(),
            total_operations,
            successful_operations,
            failed_operations,
            avg_duration_ms,
            total_records_processed: total_records,
            audit_entries: audit_count,
        }
    }
    
    /// Get performance metrics with optional filtering
    pub async fn get_performance_metrics(&self, operation_filter: Option<&str>) -> Vec<PerformanceMetric> {
        let completed = match self.completed_operations.lock() {
            Ok(completed) => completed.clone(),
            Err(_) => {
                warn!("Failed to lock completed operations for metrics");
                return Vec::new();
            }
        };
        
        completed.iter()
            .filter(|metric| {
                if let Some(filter) = operation_filter {
                    metric.operation_name.contains(filter)
                } else {
                    true
                }
            })
            .cloned()
            .collect()
    }
    
    /// Get recent audit entries
    pub async fn get_recent_audit_entries(&self, limit: usize) -> Vec<AuditEntry> {
        let audit_log = match self.audit_log.lock() {
            Ok(audit_log) => audit_log.clone(),
            Err(_) => {
                warn!("Failed to lock audit log for recent entries");
                return Vec::new();
            }
        };
        
        audit_log.iter()
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }
}

use std::sync::OnceLock;

// Global monitoring system instance using OnceLock for thread safety
static MONITORING_SYSTEM: OnceLock<Arc<MonitoringSystem>> = OnceLock::new();

/// Initialize the global monitoring system
pub fn initialize_monitoring(config: MonitoringConfig) {
    let _ = MONITORING_SYSTEM.set(Arc::new(MonitoringSystem::new(config)));
}

/// Get the global monitoring system instance
pub fn get_monitoring_system() -> Arc<MonitoringSystem> {
    MONITORING_SYSTEM.get()
        .expect("Monitoring system not initialized. Call initialize_monitoring() first.")
        .clone()
}