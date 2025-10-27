# Fireup Testing Guide

This guide provides comprehensive instructions for testing the Fireup migration tool with various scenarios and data types.

## Quick Start Testing

### 1. Environment Setup
```bash
# Start PostgreSQL
docker-compose up -d postgres

# Verify connection
psql -h localhost -p 5433 -U fireup -d fireup_dev -c "SELECT version();"
```

### 2. Basic Functionality Test
```bash
# Build the application
cargo build

# Test CLI help
cargo run -- --help

# Validate a sample backup file
cargo run -- validate --backup-file examples/backup_files/small_sample.leveldb
```

## Test Scenarios

### Scenario 1: Small Dataset Import
**Purpose**: Test basic import functionality with minimal data

```bash
# Import small sample
cargo run -- import \
  --backup-file examples/backup_files/small_sample.leveldb \
  --postgres-url postgresql://fireup:fireup_dev_password@localhost:5433/fireup_dev

# Verify import
psql -h localhost -p 5433 -U fireup -d fireup_dev -c "
  SELECT table_name, column_name, data_type 
  FROM information_schema.columns 
  WHERE table_schema = 'fireup_data'
  ORDER BY table_name, ordinal_position;
"
```

**Expected Results**:
- Tables created for users and posts collections
- Proper data type mapping
- Foreign key relationships established
- Import audit record created

### Scenario 2: Schema Analysis and DDL Generation
**Purpose**: Test schema analysis without importing data

```bash
# Analyze users collection
cargo run -- analyze \
  --backup-file examples/backup_files/users_collection.leveldb \
  --output users_schema.sql

# Review generated DDL
cat users_schema.sql

# Analyze complex nested structures
cargo run -- analyze \
  --backup-file examples/backup_files/nested_documents.leveldb \
  --output nested_schema.sql \
  --normalize-level 3
```

**Expected Results**:
- DDL file generated with CREATE TABLE statements
- Normalized schema with separate tables for nested objects
- Proper constraints and indexes
- Transformation report included

### Scenario 3: Complex E-commerce Data
**Purpose**: Test with realistic e-commerce schema

```bash
# Import product catalog
cargo run -- import \
  --backup-file examples/backup_files/products_catalog.leveldb \
  --postgres-url postgresql://fireup:fireup_dev_password@localhost:5433/fireup_dev \
  --batch-size 500

# Query imported data
psql -h localhost -p 5433 -U fireup -d fireup_dev -c "
  SELECT 
    p.name as product_name,
    c.name as category_name,
    COUNT(r.id) as review_count,
    AVG(r.rating) as avg_rating
  FROM fireup_data.products p
  LEFT JOIN fireup_data.categories c ON p.category_id = c.id
  LEFT JOIN fireup_data.reviews r ON r.product_id = p.id
  GROUP BY p.id, p.name, c.name
  ORDER BY avg_rating DESC NULLS LAST
  LIMIT 10;
"
```

**Expected Results**:
- Multiple related tables created
- Complex relationships preserved
- Aggregation queries work correctly
- Performance acceptable for dataset size

### Scenario 4: Deep Nesting Normalization
**Purpose**: Test normalization of deeply nested documents

```bash
# Import with maximum normalization
cargo run -- import \
  --backup-file examples/backup_files/nested_documents.leveldb \
  --postgres-url postgresql://fireup:fireup_dev_password@localhost:5433/fireup_dev \
  --normalize-level max

# Check normalized structure
psql -h localhost -p 5433 -U fireup -d fireup_dev -c "
  SELECT 
    table_name,
    column_name,
    data_type,
    is_nullable
  FROM information_schema.columns 
  WHERE table_schema = 'fireup_data' 
    AND table_name LIKE '%organization%'
  ORDER BY table_name, ordinal_position;
"
```

**Expected Results**:
- Deeply nested objects normalized into separate tables
- Proper foreign key relationships maintained
- No data loss during normalization
- Query performance optimized

## Performance Testing

### Large Dataset Simulation
```bash
# Test with increased batch size
cargo run -- import \
  --backup-file examples/backup_files/products_catalog.leveldb \
  --postgres-url postgresql://fireup:fireup_dev_password@localhost:5433/fireup_dev \
  --batch-size 2000 \
  --parallel-workers 4

# Monitor performance
docker stats fireup_postgres
```

### Memory Usage Testing
```bash
# Test with memory constraints
docker-compose down
docker-compose up -d postgres

# Run import with monitoring
cargo run -- import \
  --backup-file examples/backup_files/products_catalog.leveldb \
  --postgres-url postgresql://fireup:fireup_dev_password@localhost:5433/fireup_dev \
  --memory-limit 512MB
```

## Error Handling Tests

### Invalid Backup File
```bash
# Test with corrupted file
echo "invalid data" > /tmp/corrupted.leveldb
cargo run -- validate --backup-file /tmp/corrupted.leveldb

# Expected: Detailed error message with suggestions
```

### Connection Failures
```bash
# Test with wrong connection string
cargo run -- import \
  --backup-file examples/backup_files/small_sample.leveldb \
  --postgres-url postgresql://wrong:wrong@localhost:9999/wrong

# Expected: Clear connection error with troubleshooting hints
```

### Schema Conflicts
```bash
# Import same data twice to test conflict handling
cargo run -- import \
  --backup-file examples/backup_files/small_sample.leveldb \
  --postgres-url postgresql://fireup:fireup_dev_password@localhost:5433/fireup_dev

# Run again - should handle existing tables gracefully
cargo run -- import \
  --backup-file examples/backup_files/small_sample.leveldb \
  --postgres-url postgresql://fireup:fireup_dev_password@localhost:5433/fireup_dev \
  --on-conflict merge
```

## Integration Testing

### PostgreSQL Client Compatibility

#### Test with psql
```bash
# Connect and run queries
psql -h localhost -p 5433 -U fireup -d fireup_dev

# Test various SQL features
\dt fireup_data.*
SELECT * FROM fireup_data.import_summary;
EXPLAIN ANALYZE SELECT * FROM fireup_data.users LIMIT 10;
```

#### Test with pgAdmin
1. Connect to database using pgAdmin
2. Browse schema structure
3. Run sample queries
4. Export data to verify integrity

#### Test with Python
```python
# test_connection.py
import psycopg2
import json

conn = psycopg2.connect(
    host="localhost",
    port=5433,
    database="fireup_dev",
    user="fireup",
    password="fireup_dev_password"
)

with conn.cursor() as cur:
    cur.execute("SELECT COUNT(*) FROM fireup_data.import_audit")
    count = cur.fetchone()[0]
    print(f"Import operations: {count}")

conn.close()
```

### Docker Integration
```bash
# Test container lifecycle
docker-compose down
docker-compose up -d postgres
docker-compose logs postgres

# Test volume persistence
docker-compose down
docker-compose up -d postgres
psql -h localhost -p 5433 -U fireup -d fireup_dev -c "SELECT COUNT(*) FROM fireup_data.import_audit;"
```

## Automated Testing

### Unit Tests
```bash
# Run all unit tests
cargo test

# Run specific module tests
cargo test leveldb_parser
cargo test schema_analyzer
cargo test data_importer

# Run with output
cargo test -- --nocapture
```

### Integration Tests
```bash
# Run integration tests (requires PostgreSQL)
cargo test --test integration

# Run with specific test pattern
cargo test --test integration test_full_import_workflow
```

### Benchmark Tests
```bash
# Run performance benchmarks
cargo bench

# Specific benchmark
cargo bench --bench import_performance
```

## Test Data Validation

### Data Integrity Checks
```sql
-- Check for data consistency
SELECT 
  table_name,
  (xpath('/row/c/text()', query_to_xml('SELECT COUNT(*) FROM fireup_data.' || table_name, false, true, '')))[1]::text::int as row_count
FROM information_schema.tables 
WHERE table_schema = 'fireup_data' 
  AND table_type = 'BASE TABLE'
  AND table_name != 'import_audit';

-- Check foreign key integrity
SELECT 
  tc.table_name,
  tc.constraint_name,
  tc.constraint_type,
  kcu.column_name,
  ccu.table_name AS foreign_table_name,
  ccu.column_name AS foreign_column_name
FROM information_schema.table_constraints AS tc
JOIN information_schema.key_column_usage AS kcu
  ON tc.constraint_name = kcu.constraint_name
  AND tc.table_schema = kcu.table_schema
JOIN information_schema.constraint_column_usage AS ccu
  ON ccu.constraint_name = tc.constraint_name
  AND ccu.table_schema = tc.table_schema
WHERE tc.constraint_type = 'FOREIGN KEY' 
  AND tc.table_schema = 'fireup_data';
```

### Schema Validation
```sql
-- Verify expected tables exist
SELECT table_name 
FROM information_schema.tables 
WHERE table_schema = 'fireup_data' 
ORDER BY table_name;

-- Check column data types
SELECT 
  table_name,
  column_name,
  data_type,
  character_maximum_length,
  is_nullable,
  column_default
FROM information_schema.columns 
WHERE table_schema = 'fireup_data'
ORDER BY table_name, ordinal_position;
```

## Troubleshooting Test Issues

### Common Test Failures

1. **PostgreSQL Connection Issues**
   ```bash
   # Check container status
   docker-compose ps
   
   # Check logs
   docker-compose logs postgres
   
   # Restart if needed
   docker-compose restart postgres
   ```

2. **Permission Errors**
   ```bash
   # Reset database permissions
   docker-compose exec postgres psql -U fireup -d fireup_dev -c "
     GRANT ALL PRIVILEGES ON SCHEMA fireup_data TO fireup;
     GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA fireup_data TO fireup;
   "
   ```

3. **Memory Issues**
   ```bash
   # Check available memory
   free -h
   
   # Reduce batch size
   export FIREUP_MAX_BATCH_SIZE=100
   
   # Monitor memory usage
   docker stats
   ```

4. **Test Data Issues**
   ```bash
   # Clean test data
   psql -h localhost -p 5433 -U fireup -d fireup_dev -c "
     DROP SCHEMA fireup_data CASCADE;
     CREATE SCHEMA fireup_data;
   "
   
   # Restart with fresh database
   docker-compose down -v
   docker-compose up -d postgres
   ```

## Continuous Integration

### GitHub Actions Example
```yaml
# .github/workflows/test.yml
name: Test Fireup

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    
    services:
      postgres:
        image: postgres:15
        env:
          POSTGRES_DB: fireup_test
          POSTGRES_USER: fireup
          POSTGRES_PASSWORD: fireup_test_password
        options: >-
          --health-cmd pg_isready
          --health-interval 10s
          --health-timeout 5s
          --health-retries 5
        ports:
          - 5432:5432
    
    steps:
    - uses: actions/checkout@v3
    
    - name: Setup Rust
      uses: actions-rs/toolchain@v1
      with:
        toolchain: stable
        
    - name: Run tests
      run: |
        cargo test
        cargo test --test integration
      env:
        DATABASE_URL: postgresql://fireup:fireup_test_password@localhost:5432/fireup_test
```

This comprehensive testing guide ensures thorough validation of the Fireup migration tool across various scenarios and use cases.