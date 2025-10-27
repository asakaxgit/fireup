# Integration Testing Documentation

This document provides comprehensive information about the integration testing suite for the Fireup migration tool.

## Overview

The integration testing suite validates the complete end-to-end functionality of Fireup, including:

- **End-to-end backup import workflows**: Complete pipeline from LevelDB parsing to PostgreSQL import
- **PostgreSQL client tool compatibility**: Verification that standard PostgreSQL tools work with imported data
- **Docker container integration**: Testing containerized PostgreSQL backend functionality
- **Performance and monitoring**: Validation of system performance and monitoring capabilities
- **Error handling and recovery**: Testing system behavior under various failure conditions

## Test Structure

### Test Files

```
tests/
├── integration_tests.rs          # Main integration test suite
scripts/
├── run_integration_tests.sh      # Test runner script
.github/workflows/
├── integration_tests.yml         # CI/CD pipeline configuration
docs/
├── INTEGRATION_TESTING.md        # This documentation
```

### Test Categories

#### 1. End-to-End Workflow Tests (`test_end_to_end_backup_import_workflow`)

**Purpose**: Validates the complete import pipeline from backup file to PostgreSQL database.

**Test Steps**:
1. Parse LevelDB backup file using `LevelDBParser`
2. Analyze schema using `SchemaAnalyzer` 
3. Generate normalized schema with proper relationships
4. Import data using `PostgreSQLImporter`
5. Verify data integrity and audit logging

**Requirements Covered**: 1.5 (import process completion), 4.2 (PostgreSQL compatibility), 4.3 (client tool compatibility)

#### 2. PostgreSQL Client Compatibility Tests (`test_postgresql_client_tool_compatibility`)

**Purpose**: Ensures imported data works with standard PostgreSQL client tools.

**Test Features**:
- Basic SELECT queries
- JOIN operations across normalized tables
- JSONB field queries
- Array field operations
- Aggregate functions
- psql command-line tool compatibility

**Requirements Covered**: 4.2 (PostgreSQL compatibility), 4.3 (client tool compatibility)

#### 3. Docker Container Integration Tests (`test_docker_container_integration`)

**Purpose**: Validates Docker container lifecycle and integration.

**Test Areas**:
- Container startup and health checks
- Network connectivity
- Volume persistence across restarts
- Resource usage monitoring
- Container restart recovery

**Requirements Covered**: 4.2 (PostgreSQL compatibility), 4.3 (client tool compatibility)

#### 4. Complex Schema Normalization Tests (`test_complex_schema_normalization_workflow`)

**Purpose**: Tests normalization of deeply nested document structures.

**Test Validation**:
- Multiple normalized tables creation
- Foreign key relationship establishment
- Referential integrity maintenance
- Complex data structure handling

**Requirements Covered**: 1.5 (import process), 4.2 (PostgreSQL compatibility)

#### 5. Error Handling and Recovery Tests (`test_error_handling_and_recovery`)

**Purpose**: Validates system behavior under error conditions.

**Error Scenarios**:
- Invalid backup file handling
- Database connection failures
- Schema conflict resolution
- Graceful error reporting

**Requirements Covered**: 1.5 (error handling), 4.2 (connection management)

#### 6. Performance and Monitoring Tests (`test_performance_and_monitoring`)

**Purpose**: Validates system performance and monitoring capabilities.

**Performance Metrics**:
- Parse operation timing
- Import throughput measurement
- Memory usage monitoring
- Audit log generation

**Requirements Covered**: 1.5 (performance), 4.2 (monitoring)

## Running Tests

### Local Development

#### Prerequisites

1. **Docker and Docker Compose**:
   ```bash
   docker --version
   docker-compose --version
   ```

2. **Rust Development Environment**:
   ```bash
   rustc --version
   cargo --version
   ```

3. **PostgreSQL Client Tools** (optional, for manual testing):
   ```bash
   psql --version
   ```

#### Quick Test Run

```bash
# Start PostgreSQL container
docker-compose up -d postgres

# Run integration tests
cargo test --test integration_tests

# Run with output
cargo test --test integration_tests -- --nocapture
```

#### Comprehensive Test Suite

```bash
# Run the complete test suite
./scripts/run_integration_tests.sh
```

This script performs:
- Environment validation
- Container startup and health checks
- Project building
- Unit test execution
- Integration test execution
- CLI functionality testing
- Client compatibility verification
- Container health validation
- Cleanup and reporting

### Continuous Integration

The project uses GitHub Actions for automated testing:

#### Workflows

1. **Integration Tests** (`.github/workflows/integration_tests.yml`):
   - Runs on push/PR to main/develop branches
   - Tests against PostgreSQL 15
   - Includes unit tests, integration tests, and CLI testing
   - Generates coverage reports

2. **Docker Integration**:
   - Tests Docker container functionality
   - Validates container persistence
   - Checks container health and networking

3. **Performance Tests**:
   - Benchmarks import performance
   - Tests concurrent operations
   - Monitors resource usage

4. **Compatibility Tests**:
   - Tests across multiple OS (Ubuntu, macOS)
   - Tests multiple Rust versions (stable, beta)

## Test Data

### Mock Data

When backup files are not available, tests use mock `FirestoreDocument` data:

```rust
FirestoreDocument {
    id: "user1".to_string(),
    collection: "users".to_string(),
    data: {
        "name": "John Doe",
        "email": "john@example.com",
        "age": 30
    },
    // ...
}
```

### Example Backup Files

The test suite can use example backup files from `examples/backup_files/`:

- `small_sample.leveldb`: Basic test data (~500KB)
- `users_collection.leveldb`: User management data (~2MB)
- `products_catalog.leveldb`: E-commerce data (~5MB)
- `nested_documents.leveldb`: Complex nested structures (~1MB)

**Note**: These files are not included in the repository. Tests will skip file-dependent tests if files are missing.

## Test Environment Configuration

### Environment Variables

- `TEST_DATABASE_URL`: PostgreSQL connection string for tests
- `POSTGRES_PORT`: PostgreSQL port (default: 5433)
- `POSTGRES_DB`: Test database name
- `POSTGRES_USER`: Database user
- `POSTGRES_PASSWORD`: Database password

### Docker Configuration

The test environment uses the same Docker Compose configuration as development:

```yaml
services:
  postgres:
    image: postgres:15
    environment:
      POSTGRES_DB: fireup_dev
      POSTGRES_USER: fireup
      POSTGRES_PASSWORD: fireup_dev_password
    ports:
      - "5433:5432"
```

## Test Assertions and Validation

### Data Integrity Checks

```rust
// Verify documents were parsed
assert!(!parse_result.documents.is_empty(), "Should parse documents");

// Verify schema normalization
assert!(!normalized_schema.tables.is_empty(), "Should generate tables");

// Verify data import
assert!(import_result.success, "Import should succeed");
assert!(import_result.records_imported > 0, "Should import records");
```

### PostgreSQL Compatibility Checks

```sql
-- Verify table creation
SELECT table_name FROM information_schema.tables 
WHERE table_schema = 'fireup_data';

-- Verify foreign key constraints
SELECT constraint_name, constraint_type 
FROM information_schema.table_constraints 
WHERE constraint_type = 'FOREIGN KEY';

-- Verify data integrity
SELECT COUNT(*) FROM fireup_data.users;
```

### Performance Benchmarks

```rust
let start_time = std::time::Instant::now();
// ... operation ...
let duration = start_time.elapsed();

assert!(duration < Duration::from_secs(60), "Should complete within 60s");

let throughput = records_imported as f64 / duration.as_secs_f64();
assert!(throughput > 10.0, "Should achieve reasonable throughput");
```

## Troubleshooting

### Common Issues

#### 1. PostgreSQL Connection Failures

**Symptoms**: Tests fail with connection errors

**Solutions**:
```bash
# Check container status
docker-compose ps

# Check container logs
docker-compose logs postgres

# Restart container
docker-compose restart postgres
```

#### 2. Test Database Permission Issues

**Symptoms**: Permission denied errors during tests

**Solutions**:
```bash
# Reset database permissions
docker exec fireup_postgres psql -U fireup -d fireup_dev -c "
  GRANT ALL PRIVILEGES ON SCHEMA fireup_data TO fireup;
  GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA fireup_data TO fireup;
"
```

#### 3. Missing Backup Files

**Symptoms**: Tests skip with "backup file not found" messages

**Solutions**:
- Tests will use mock data automatically
- For full testing, create or obtain sample backup files
- Place files in `examples/backup_files/` directory

#### 4. Memory Issues During Large Tests

**Symptoms**: Out of memory errors or slow performance

**Solutions**:
```bash
# Reduce batch sizes
export FIREUP_MAX_BATCH_SIZE=100

# Monitor memory usage
docker stats fireup_postgres

# Increase Docker memory limits
# Edit docker-compose.yml to increase memory limits
```

### Debug Mode

Run tests with additional debugging:

```bash
# Enable debug logging
RUST_LOG=debug cargo test --test integration_tests -- --nocapture

# Run specific test
cargo test --test integration_tests test_end_to_end_backup_import_workflow -- --nocapture

# Run with timing information
cargo test --test integration_tests -- --nocapture --show-output
```

## Test Metrics and Reporting

### Coverage Reports

Integration tests generate coverage reports using `cargo-tarpaulin`:

```bash
# Install tarpaulin
cargo install cargo-tarpaulin

# Generate coverage report
cargo tarpaulin --out html --output-dir coverage/

# View report
open coverage/tarpaulin-report.html
```

### Performance Metrics

Tests track and report:
- Parse operation duration
- Schema analysis time
- Import throughput (records/second)
- Memory usage
- Database operation timing

### Test Reports

The test runner generates comprehensive reports including:
- Test execution summary
- Performance benchmarks
- Error logs and diagnostics
- System information
- Database statistics

## Best Practices

### Writing Integration Tests

1. **Test Independence**: Each test should be independent and not rely on other tests
2. **Cleanup**: Always clean up test data after test completion
3. **Error Handling**: Test both success and failure scenarios
4. **Performance**: Include performance assertions for critical operations
5. **Documentation**: Document test purpose and requirements coverage

### Test Data Management

1. **Mock Data**: Use mock data when possible to reduce dependencies
2. **File Dependencies**: Make tests resilient to missing backup files
3. **Data Cleanup**: Always clean up test databases after tests
4. **Isolation**: Use separate test databases to avoid conflicts

### CI/CD Integration

1. **Fast Feedback**: Keep test execution time reasonable
2. **Parallel Execution**: Run independent tests in parallel
3. **Artifact Collection**: Collect test artifacts for debugging
4. **Coverage Tracking**: Monitor test coverage over time

## Future Enhancements

### Planned Test Improvements

1. **Load Testing**: Add tests for large dataset imports (>1GB)
2. **Stress Testing**: Test system behavior under resource constraints
3. **Security Testing**: Validate security measures and access controls
4. **Multi-Version Testing**: Test against multiple PostgreSQL versions
5. **Network Failure Testing**: Test behavior under network interruptions

### Test Automation

1. **Scheduled Testing**: Run comprehensive tests on schedule
2. **Performance Regression Detection**: Automated performance monitoring
3. **Test Data Generation**: Automated generation of test backup files
4. **Cross-Platform Testing**: Extended OS and architecture coverage

This integration testing suite ensures the reliability, performance, and compatibility of the Fireup migration tool across various scenarios and environments.