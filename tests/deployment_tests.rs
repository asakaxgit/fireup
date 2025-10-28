use anyhow::Result;
use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;
use tokio::time::sleep;
use tokio_postgres::NoTls;

/// Test configuration for deployment tests
struct DeploymentTestConfig {
    postgres_host: String,
    postgres_port: String,
    postgres_user: String,
    postgres_password: String,
    postgres_db: String,
}

impl DeploymentTestConfig {
    fn from_env() -> Self {
        Self {
            postgres_host: std::env::var("POSTGRES_HOST").unwrap_or_else(|_| "localhost".to_string()),
            postgres_port: std::env::var("POSTGRES_PORT").unwrap_or_else(|_| "5433".to_string()),
            postgres_user: std::env::var("POSTGRES_USER").unwrap_or_else(|_| "fireup".to_string()),
            postgres_password: std::env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "fireup_dev_password".to_string()),
            postgres_db: std::env::var("POSTGRES_DB").unwrap_or_else(|_| "fireup_dev".to_string()),
        }
    }

    fn connection_url(&self) -> String {
        format!(
            "postgresql://{}:{}@{}:{}/{}",
            self.postgres_user, self.postgres_password, self.postgres_host, self.postgres_port, self.postgres_db
        )
    }
}

/// Docker container management for deployment tests
struct DockerManager;

impl DockerManager {
    /// Start PostgreSQL container using docker-compose
    fn start_postgres_container() -> Result<()> {
        println!("Starting PostgreSQL container...");
        
        let status = Command::new("docker-compose")
            .args(&["up", "-d", "postgres"])
            .status()?;

        if !status.success() {
            return Err(anyhow::anyhow!("Failed to start PostgreSQL container"));
        }

        Ok(())
    }

    /// Stop PostgreSQL container
    fn stop_postgres_container() -> Result<()> {
        println!("Stopping PostgreSQL container...");
        
        let status = Command::new("docker-compose")
            .args(&["stop", "postgres"])
            .status()?;

        if !status.success() {
            return Err(anyhow::anyhow!("Failed to stop PostgreSQL container"));
        }

        Ok(())
    }

    /// Check if PostgreSQL container is running
    fn is_postgres_running() -> Result<bool> {
        let output = Command::new("docker")
            .args(&["ps", "--filter", "name=fireup_postgres", "--format", "{{.Names}}"])
            .output()?;

        let container_names = String::from_utf8_lossy(&output.stdout);
        Ok(container_names.trim().contains("fireup_postgres"))
    }

    /// Get container status
    fn get_container_status() -> Result<String> {
        let output = Command::new("docker")
            .args(&["ps", "--filter", "name=fireup_postgres", "--format", "{{.Status}}"])
            .output()?;

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Check container health using pg_isready
    fn check_postgres_health(config: &DeploymentTestConfig) -> Result<bool> {
        let output = Command::new("docker")
            .args(&[
                "exec", "fireup_postgres",
                "pg_isready", 
                "-h", "localhost",
                "-p", "5432",
                "-U", &config.postgres_user,
                "-d", &config.postgres_db
            ])
            .output()?;

        Ok(output.status.success())
    }

    /// Wait for PostgreSQL to be ready with timeout
    async fn wait_for_postgres_ready(config: &DeploymentTestConfig, timeout_secs: u64) -> Result<()> {
        let start_time = std::time::Instant::now();
        
        while start_time.elapsed().as_secs() < timeout_secs {
            if Self::check_postgres_health(config)? {
                return Ok(());
            }
            sleep(Duration::from_secs(2)).await;
        }

        Err(anyhow::anyhow!("PostgreSQL container did not become ready within {} seconds", timeout_secs))
    }

    /// Get container logs
    fn get_container_logs() -> Result<String> {
        let output = Command::new("docker")
            .args(&["logs", "fireup_postgres", "--tail", "50"])
            .output()?;

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Restart PostgreSQL container
    fn restart_postgres_container() -> Result<()> {
        println!("Restarting PostgreSQL container...");
        
        let status = Command::new("docker-compose")
            .args(&["restart", "postgres"])
            .status()?;

        if !status.success() {
            return Err(anyhow::anyhow!("Failed to restart PostgreSQL container"));
        }

        Ok(())
    }
}

#[tokio::test]
async fn test_docker_container_startup_and_connectivity() -> Result<()> {
    let config = DeploymentTestConfig::from_env();

    // Test 1: Start container if not running
    if !DockerManager::is_postgres_running()? {
        DockerManager::start_postgres_container()?;
    }

    // Test 2: Verify container is running
    assert!(DockerManager::is_postgres_running()?, "PostgreSQL container should be running");

    // Test 3: Check container status
    let status = DockerManager::get_container_status()?;
    assert!(status.contains("Up"), "Container status should indicate it's running: {}", status);

    // Test 4: Wait for PostgreSQL to be ready
    DockerManager::wait_for_postgres_ready(&config, 60).await?;

    // Test 5: Verify health check
    assert!(DockerManager::check_postgres_health(&config)?, "PostgreSQL health check should pass");

    // Test 6: Test container networking
    let network_test = Command::new("docker")
        .args(&[
            "exec", "fireup_postgres",
            "pg_isready", "-h", "localhost", "-p", "5432", "-U", &config.postgres_user
        ])
        .output()?;

    assert!(network_test.status.success(), "Container internal networking should work");

    // Test 7: Test port mapping
    let port_test = Command::new("docker")
        .args(&[
            "port", "fireup_postgres", "5432"
        ])
        .output()?;

    let port_mapping = String::from_utf8_lossy(&port_test.stdout);
    assert!(port_mapping.contains(&config.postgres_port), "Port mapping should be configured correctly");

    Ok(())
}

#[tokio::test]
async fn test_postgresql_client_connections() -> Result<()> {
    let config = DeploymentTestConfig::from_env();

    // Ensure container is running and ready
    if !DockerManager::is_postgres_running()? {
        DockerManager::start_postgres_container()?;
    }
    DockerManager::wait_for_postgres_ready(&config, 60).await?;

    // Test 1: Basic connection using tokio-postgres
    let (client, connection) = tokio_postgres::connect(&config.connection_url(), NoTls).await?;
    
    // Spawn connection task
    let connection_handle = tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Connection error: {}", e);
        }
    });

    // Test 2: Execute basic query
    let rows = client.query("SELECT version()", &[]).await?;
    assert_eq!(rows.len(), 1, "Should return PostgreSQL version");
    
    let version: String = rows[0].get(0);
    assert!(version.contains("PostgreSQL"), "Should return PostgreSQL version string");

    // Test 3: Test database operations
    client.execute("CREATE SCHEMA IF NOT EXISTS test_deployment", &[]).await?;
    
    client.execute("
        CREATE TABLE IF NOT EXISTS test_deployment.connection_test (
            id SERIAL PRIMARY KEY,
            test_data TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT NOW()
        )
    ", &[]).await?;

    // Test 4: Insert and query data
    client.execute("
        INSERT INTO test_deployment.connection_test (test_data) 
        VALUES ('deployment_test_data')
    ", &[]).await?;

    let test_rows = client.query("
        SELECT test_data FROM test_deployment.connection_test 
        WHERE test_data = 'deployment_test_data'
    ", &[]).await?;

    assert_eq!(test_rows.len(), 1, "Should find inserted test data");

    // Test 5: Test psql command-line client
    let psql_test = Command::new("docker")
        .args(&[
            "exec", "fireup_postgres",
            "psql", "-U", &config.postgres_user, "-d", &config.postgres_db, "-c",
            "SELECT COUNT(*) FROM test_deployment.connection_test;"
        ])
        .output()?;

    assert!(psql_test.status.success(), "psql command should execute successfully");
    let psql_output = String::from_utf8_lossy(&psql_test.stdout);
    assert!(psql_output.contains("1"), "psql should return correct count");

    // Test 6: Test connection with different clients (pg_dump)
    let pg_dump_test = Command::new("docker")
        .args(&[
            "exec", "fireup_postgres",
            "pg_dump", "-U", &config.postgres_user, "-d", &config.postgres_db,
            "--schema=test_deployment", "--schema-only"
        ])
        .output()?;

    assert!(pg_dump_test.status.success(), "pg_dump should execute successfully");
    let dump_output = String::from_utf8_lossy(&pg_dump_test.stdout);
    assert!(dump_output.contains("connection_test"), "pg_dump should include test table");

    // Test 7: Test concurrent connections
    let mut connection_tasks = vec![];
    
    for i in 0..5 {
        let connection_url = config.connection_url();
        let task = tokio::spawn(async move {
            let (client, connection) = tokio_postgres::connect(&connection_url, NoTls).await?;
            
            tokio::spawn(async move {
                if let Err(e) = connection.await {
                    eprintln!("Connection {} error: {}", i, e);
                }
            });

            let rows = client.query("SELECT $1::text", &[&format!("connection_{}", i)]).await?;
            let result: String = rows[0].get(0);
            
            anyhow::Ok(result)
        });
        
        connection_tasks.push(task);
    }

    // Wait for all connections to complete
    for (i, task) in connection_tasks.into_iter().enumerate() {
        let result = task.await??;
        assert_eq!(result, format!("connection_{}", i), "Concurrent connection {} should work", i);
    }

    // Cleanup
    client.execute("DROP SCHEMA test_deployment CASCADE", &[]).await?;
    
    // Close connection
    connection_handle.abort();

    Ok(())
}

#[tokio::test]
async fn test_environment_variable_configuration() -> Result<()> {
    // Test 1: Verify default environment variables are working
    let default_config = DeploymentTestConfig::from_env();
    
    // Ensure container is running with current config
    if !DockerManager::is_postgres_running()? {
        DockerManager::start_postgres_container()?;
    }
    DockerManager::wait_for_postgres_ready(&default_config, 60).await?;

    // Test connection with default config
    let (client, connection) = tokio_postgres::connect(&default_config.connection_url(), NoTls).await?;
    
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Connection error: {}", e);
        }
    });

    // Verify database name
    let db_rows = client.query("SELECT current_database()", &[]).await?;
    let current_db: String = db_rows[0].get(0);
    assert_eq!(current_db, default_config.postgres_db, "Should connect to correct database");

    // Verify user
    let user_rows = client.query("SELECT current_user", &[]).await?;
    let current_user: String = user_rows[0].get(0);
    assert_eq!(current_user, default_config.postgres_user, "Should connect as correct user");

    // Test 2: Verify container environment variables
    let env_check = Command::new("docker")
        .args(&[
            "exec", "fireup_postgres",
            "printenv", "POSTGRES_DB"
        ])
        .output()?;

    assert!(env_check.status.success(), "Should be able to read container environment");
    let container_db = String::from_utf8_lossy(&env_check.stdout).trim().to_string();
    assert_eq!(container_db, default_config.postgres_db, "Container should have correct POSTGRES_DB");

    // Test 3: Test with custom environment variables
    let custom_env_vars = HashMap::from([
        ("POSTGRES_HOST", "localhost"),
        ("POSTGRES_PORT", "5433"),
        ("POSTGRES_USER", "fireup"),
        ("POSTGRES_PASSWORD", "fireup_dev_password"),
        ("POSTGRES_DB", "fireup_dev"),
    ]);

    // Verify each environment variable can be read
    for (key, expected_value) in &custom_env_vars {
        let actual_value = std::env::var(key).unwrap_or_else(|_| expected_value.to_string());
        println!("Environment variable {}: {} (expected: {})", key, actual_value, expected_value);
    }

    // Test 4: Test PostgreSQL configuration parameters
    let config_rows = client.query("
        SELECT name, setting 
        FROM pg_settings 
        WHERE name IN ('log_statement', 'log_min_duration_statement', 'shared_preload_libraries')
        ORDER BY name
    ", &[]).await?;

    assert!(!config_rows.is_empty(), "Should be able to query PostgreSQL configuration");

    // Test 5: Test logging configuration
    let log_statement_row = client.query("
        SELECT setting FROM pg_settings WHERE name = 'log_statement'
    ", &[]).await?;

    if !log_statement_row.is_empty() {
        let log_setting: String = log_statement_row[0].get(0);
        println!("PostgreSQL log_statement setting: {}", log_setting);
    }

    // Test 6: Test resource limits (if configured)
    let memory_check = Command::new("docker")
        .args(&[
            "stats", "fireup_postgres", "--no-stream", "--format", 
            "table {{.Container}}\t{{.MemUsage}}\t{{.MemPerc}}"
        ])
        .output()?;

    if memory_check.status.success() {
        let stats_output = String::from_utf8_lossy(&memory_check.stdout);
        println!("Container resource usage: {}", stats_output);
        assert!(stats_output.contains("fireup_postgres"), "Should show container stats");
    }

    // Test 7: Test volume configuration
    let volume_check = Command::new("docker")
        .args(&[
            "inspect", "fireup_postgres", "--format", 
            "{{range .Mounts}}{{.Source}}:{{.Destination}} {{end}}"
        ])
        .output()?;

    assert!(volume_check.status.success(), "Should be able to inspect container volumes");
    let volume_info = String::from_utf8_lossy(&volume_check.stdout);
    assert!(volume_info.contains("/var/lib/postgresql/data"), "Should have PostgreSQL data volume");

    // Test 8: Test network configuration
    let network_check = Command::new("docker")
        .args(&[
            "inspect", "fireup_postgres", "--format", 
            "{{range $k, $v := .NetworkSettings.Networks}}{{$k}} {{end}}"
        ])
        .output()?;

    assert!(network_check.status.success(), "Should be able to inspect container networks");
    let network_info = String::from_utf8_lossy(&network_check.stdout);
    println!("Container networks: {}", network_info);

    Ok(())
}

#[tokio::test]
async fn test_container_persistence_and_recovery() -> Result<()> {
    let config = DeploymentTestConfig::from_env();

    // Ensure container is running
    if !DockerManager::is_postgres_running()? {
        DockerManager::start_postgres_container()?;
    }
    DockerManager::wait_for_postgres_ready(&config, 60).await?;

    // Test 1: Create test data
    let (client, connection) = tokio_postgres::connect(&config.connection_url(), NoTls).await?;
    
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Connection error: {}", e);
        }
    });

    client.execute("CREATE SCHEMA IF NOT EXISTS persistence_test", &[]).await?;
    
    client.execute("
        CREATE TABLE persistence_test.recovery_data (
            id SERIAL PRIMARY KEY,
            data TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT NOW()
        )
    ", &[]).await?;

    client.execute("
        INSERT INTO persistence_test.recovery_data (data) 
        VALUES ('before_restart'), ('test_data_1'), ('test_data_2')
    ", &[]).await?;

    // Verify data exists
    let initial_rows = client.query("
        SELECT COUNT(*) FROM persistence_test.recovery_data
    ", &[]).await?;
    let initial_count: i64 = initial_rows[0].get(0);
    assert_eq!(initial_count, 3, "Should have 3 initial records");

    // Test 2: Restart container
    DockerManager::restart_postgres_container()?;
    DockerManager::wait_for_postgres_ready(&config, 60).await?;

    // Test 3: Reconnect and verify data persistence
    let (client_after_restart, connection_after_restart) = tokio_postgres::connect(&config.connection_url(), NoTls).await?;
    
    tokio::spawn(async move {
        if let Err(e) = connection_after_restart.await {
            eprintln!("Connection error after restart: {}", e);
        }
    });

    let persisted_rows = client_after_restart.query("
        SELECT COUNT(*) FROM persistence_test.recovery_data
    ", &[]).await?;
    let persisted_count: i64 = persisted_rows[0].get(0);
    assert_eq!(persisted_count, 3, "Data should persist after container restart");

    // Test 4: Verify specific data
    let data_rows = client_after_restart.query("
        SELECT data FROM persistence_test.recovery_data 
        WHERE data = 'before_restart'
    ", &[]).await?;
    assert_eq!(data_rows.len(), 1, "Specific data should persist");

    // Test 5: Test container stop/start cycle
    DockerManager::stop_postgres_container()?;
    
    // Verify container is stopped
    assert!(!DockerManager::is_postgres_running()?, "Container should be stopped");

    // Start container again
    DockerManager::start_postgres_container()?;
    DockerManager::wait_for_postgres_ready(&config, 60).await?;

    // Test 6: Verify data after stop/start cycle
    let (client_after_cycle, connection_after_cycle) = tokio_postgres::connect(&config.connection_url(), NoTls).await?;
    
    tokio::spawn(async move {
        if let Err(e) = connection_after_cycle.await {
            eprintln!("Connection error after stop/start cycle: {}", e);
        }
    });

    let final_rows = client_after_cycle.query("
        SELECT COUNT(*) FROM persistence_test.recovery_data
    ", &[]).await?;
    let final_count: i64 = final_rows[0].get(0);
    assert_eq!(final_count, 3, "Data should persist after stop/start cycle");

    // Cleanup
    client_after_cycle.execute("DROP SCHEMA persistence_test CASCADE", &[]).await?;

    Ok(())
}

#[tokio::test]
async fn test_container_health_and_monitoring() -> Result<()> {
    let config = DeploymentTestConfig::from_env();

    // Ensure container is running
    if !DockerManager::is_postgres_running()? {
        DockerManager::start_postgres_container()?;
    }
    DockerManager::wait_for_postgres_ready(&config, 60).await?;

    // Test 1: Health check endpoint
    assert!(DockerManager::check_postgres_health(&config)?, "Health check should pass");

    // Test 2: Container logs
    let logs = DockerManager::get_container_logs()?;
    assert!(!logs.is_empty(), "Should be able to retrieve container logs");
    println!("Container logs (last 50 lines): {}", logs);

    // Test 3: PostgreSQL process status
    let process_check = Command::new("docker")
        .args(&[
            "exec", "fireup_postgres",
            "ps", "aux"
        ])
        .output()?;

    assert!(process_check.status.success(), "Should be able to check processes");
    let process_output = String::from_utf8_lossy(&process_check.stdout);
    assert!(process_output.contains("postgres"), "PostgreSQL process should be running");

    // Test 4: Database connection limits
    let (client, connection) = tokio_postgres::connect(&config.connection_url(), NoTls).await?;
    
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Connection error: {}", e);
        }
    });

    let connection_info = client.query("
        SELECT 
            setting as max_connections,
            (SELECT count(*) FROM pg_stat_activity) as current_connections
        FROM pg_settings 
        WHERE name = 'max_connections'
    ", &[]).await?;

    assert_eq!(connection_info.len(), 1, "Should get connection information");
    let max_connections: String = connection_info[0].get(0);
    let current_connections: i64 = connection_info[0].get(1);
    
    println!("Max connections: {}, Current connections: {}", max_connections, current_connections);
    assert!(current_connections > 0, "Should have active connections");

    // Test 5: Database size and statistics
    let db_stats = client.query("
        SELECT 
            pg_database_size(current_database()) as db_size,
            (SELECT count(*) FROM pg_stat_user_tables) as user_tables
    ", &[]).await?;

    assert_eq!(db_stats.len(), 1, "Should get database statistics");
    let db_size: i64 = db_stats[0].get(0);
    println!("Database size: {} bytes", db_size);
    assert!(db_size > 0, "Database should have some size");

    // Test 6: Container resource usage
    let resource_check = Command::new("docker")
        .args(&[
            "exec", "fireup_postgres",
            "cat", "/proc/meminfo"
        ])
        .output()?;

    if resource_check.status.success() {
        let meminfo = String::from_utf8_lossy(&resource_check.stdout);
        assert!(meminfo.contains("MemTotal"), "Should be able to read memory information");
    }

    Ok(())
}