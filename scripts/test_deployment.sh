#!/bin/bash

# Test deployment and configuration script
# This script tests Docker container startup, connectivity, and environment variables

set -e

echo "=== Fireup Deployment Tests ==="

# Test 1: Docker container startup and connectivity
echo "Test 1: Testing Docker container startup and connectivity..."

# Check if Docker is running
if ! docker info > /dev/null 2>&1; then
    echo "ERROR: Docker is not running. Please start Docker and try again."
    exit 1
fi

# Start PostgreSQL container
echo "Starting PostgreSQL container..."
docker-compose up -d postgres

# Wait for container to be ready
echo "Waiting for PostgreSQL to be ready..."
for i in {1..30}; do
    if docker exec fireup_postgres pg_isready -U fireup -d fireup_dev > /dev/null 2>&1; then
        echo "PostgreSQL is ready!"
        break
    fi
    if [ $i -eq 30 ]; then
        echo "ERROR: PostgreSQL container did not become ready within 30 seconds"
        docker logs fireup_postgres
        exit 1
    fi
    sleep 1
done

# Test 2: PostgreSQL client connections
echo "Test 2: Testing PostgreSQL client connections..."

# Test basic connection
echo "Testing basic connection..."
docker exec fireup_postgres psql -U fireup -d fireup_dev -c "SELECT version();" > /dev/null
if [ $? -eq 0 ]; then
    echo "✓ Basic connection test passed"
else
    echo "✗ Basic connection test failed"
    exit 1
fi

# Test database operations
echo "Testing database operations..."
docker exec fireup_postgres psql -U fireup -d fireup_dev -c "
    CREATE SCHEMA IF NOT EXISTS test_deployment;
    CREATE TABLE IF NOT EXISTS test_deployment.connection_test (
        id SERIAL PRIMARY KEY,
        test_data TEXT NOT NULL,
        created_at TIMESTAMP DEFAULT NOW()
    );
    INSERT INTO test_deployment.connection_test (test_data) VALUES ('deployment_test_data');
    SELECT COUNT(*) FROM test_deployment.connection_test;
" > /dev/null

if [ $? -eq 0 ]; then
    echo "✓ Database operations test passed"
else
    echo "✗ Database operations test failed"
    exit 1
fi

# Test concurrent connections
echo "Testing concurrent connections..."
for i in {1..5}; do
    docker exec fireup_postgres psql -U fireup -d fireup_dev -c "SELECT 'connection_$i';" > /dev/null &
done
wait

if [ $? -eq 0 ]; then
    echo "✓ Concurrent connections test passed"
else
    echo "✗ Concurrent connections test failed"
    exit 1
fi

# Test 3: Environment variable configuration
echo "Test 3: Testing environment variable configuration..."

# Check container environment variables
echo "Checking container environment variables..."
POSTGRES_DB=$(docker exec fireup_postgres printenv POSTGRES_DB)
POSTGRES_USER=$(docker exec fireup_postgres printenv POSTGRES_USER)

if [ "$POSTGRES_DB" = "fireup_dev" ] && [ "$POSTGRES_USER" = "fireup" ]; then
    echo "✓ Environment variables test passed"
else
    echo "✗ Environment variables test failed"
    echo "Expected POSTGRES_DB=fireup_dev, got: $POSTGRES_DB"
    echo "Expected POSTGRES_USER=fireup, got: $POSTGRES_USER"
    exit 1
fi

# Test PostgreSQL configuration
echo "Testing PostgreSQL configuration..."
docker exec fireup_postgres psql -U fireup -d fireup_dev -c "
    SELECT name, setting 
    FROM pg_settings 
    WHERE name IN ('log_statement', 'shared_preload_libraries')
    ORDER BY name;
" > /dev/null

if [ $? -eq 0 ]; then
    echo "✓ PostgreSQL configuration test passed"
else
    echo "✗ PostgreSQL configuration test failed"
    exit 1
fi

# Test 4: Container persistence and recovery
echo "Test 4: Testing container persistence and recovery..."

# Create test data
echo "Creating test data..."
docker exec fireup_postgres psql -U fireup -d fireup_dev -c "
    CREATE SCHEMA IF NOT EXISTS persistence_test;
    CREATE TABLE IF NOT EXISTS persistence_test.recovery_data (
        id SERIAL PRIMARY KEY,
        data TEXT NOT NULL,
        created_at TIMESTAMP DEFAULT NOW()
    );
    INSERT INTO persistence_test.recovery_data (data) 
    VALUES ('before_restart'), ('test_data_1'), ('test_data_2');
"

# Restart container
echo "Restarting container..."
docker-compose restart postgres

# Wait for container to be ready after restart
echo "Waiting for PostgreSQL to be ready after restart..."
for i in {1..30}; do
    if docker exec fireup_postgres pg_isready -U fireup -d fireup_dev > /dev/null 2>&1; then
        echo "PostgreSQL is ready after restart!"
        break
    fi
    if [ $i -eq 30 ]; then
        echo "ERROR: PostgreSQL container did not become ready after restart"
        exit 1
    fi
    sleep 1
done

# Verify data persistence
echo "Verifying data persistence..."
COUNT=$(docker exec fireup_postgres psql -U fireup -d fireup_dev -t -c "SELECT COUNT(*) FROM persistence_test.recovery_data;" | tr -d ' ')

if [ "$COUNT" = "3" ]; then
    echo "✓ Container persistence test passed"
else
    echo "✗ Container persistence test failed"
    echo "Expected 3 records, got: $COUNT"
    exit 1
fi

# Test 5: Container health and monitoring
echo "Test 5: Testing container health and monitoring..."

# Check container status
echo "Checking container status..."
STATUS=$(docker ps --filter "name=fireup_postgres" --format "{{.Status}}")
if [[ "$STATUS" == *"Up"* ]]; then
    echo "✓ Container status test passed"
else
    echo "✗ Container status test failed"
    echo "Container status: $STATUS"
    exit 1
fi

# Check container logs
echo "Checking container logs..."
docker logs fireup_postgres --tail 10 > /dev/null
if [ $? -eq 0 ]; then
    echo "✓ Container logs test passed"
else
    echo "✗ Container logs test failed"
    exit 1
fi

# Check resource usage
echo "Checking resource usage..."
docker stats fireup_postgres --no-stream --format "table {{.Container}}\t{{.MemUsage}}\t{{.MemPerc}}" > /dev/null
if [ $? -eq 0 ]; then
    echo "✓ Resource usage test passed"
else
    echo "✗ Resource usage test failed"
    exit 1
fi

# Test 6: Network and volume configuration
echo "Test 6: Testing network and volume configuration..."

# Check port mapping
echo "Checking port mapping..."
PORT_MAPPING=$(docker port fireup_postgres 5432)
if [[ "$PORT_MAPPING" == *"5433"* ]]; then
    echo "✓ Port mapping test passed"
else
    echo "✗ Port mapping test failed"
    echo "Port mapping: $PORT_MAPPING"
    exit 1
fi

# Check volume configuration
echo "Checking volume configuration..."
VOLUMES=$(docker inspect fireup_postgres --format '{{range .Mounts}}{{.Source}}:{{.Destination}} {{end}}')
if [[ "$VOLUMES" == *"/var/lib/postgresql/data"* ]]; then
    echo "✓ Volume configuration test passed"
else
    echo "✗ Volume configuration test failed"
    echo "Volumes: $VOLUMES"
    exit 1
fi

# Cleanup test data
echo "Cleaning up test data..."
docker exec fireup_postgres psql -U fireup -d fireup_dev -c "
    DROP SCHEMA IF EXISTS test_deployment CASCADE;
    DROP SCHEMA IF EXISTS persistence_test CASCADE;
" > /dev/null

echo ""
echo "=== All Deployment Tests Passed! ==="
echo ""
echo "Summary:"
echo "✓ Docker container startup and connectivity"
echo "✓ PostgreSQL client connections"
echo "✓ Environment variable configuration"
echo "✓ Container persistence and recovery"
echo "✓ Container health and monitoring"
echo "✓ Network and volume configuration"
echo ""
echo "The Fireup deployment is working correctly!"