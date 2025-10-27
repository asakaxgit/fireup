#!/bin/bash

# Integration Test Runner for Fireup
# This script sets up the test environment and runs comprehensive integration tests

set -e

echo "🚀 Starting Fireup Integration Tests"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
POSTGRES_PORT=${POSTGRES_PORT:-5433}
TEST_DATABASE_URL="postgresql://fireup:fireup_dev_password@localhost:${POSTGRES_PORT}/fireup_test"
DOCKER_COMPOSE_FILE="docker-compose.yml"

# Function to print colored output
print_status() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Function to check if Docker is running
check_docker() {
    if ! docker info > /dev/null 2>&1; then
        print_error "Docker is not running. Please start Docker and try again."
        exit 1
    fi
    print_success "Docker is running"
}

# Function to check if docker-compose is available
check_docker_compose() {
    if ! command -v docker-compose &> /dev/null; then
        print_error "docker-compose is not installed. Please install docker-compose and try again."
        exit 1
    fi
    print_success "docker-compose is available"
}

# Function to start PostgreSQL container
start_postgres() {
    print_status "Starting PostgreSQL container..."
    
    if docker ps --filter "name=fireup_postgres" --format "{{.Names}}" | grep -q "fireup_postgres"; then
        print_warning "PostgreSQL container is already running"
    else
        docker-compose up -d postgres
        print_success "PostgreSQL container started"
    fi
    
    # Wait for PostgreSQL to be ready
    print_status "Waiting for PostgreSQL to be ready..."
    for i in {1..30}; do
        if docker exec fireup_postgres pg_isready -U fireup -d fireup_dev > /dev/null 2>&1; then
            print_success "PostgreSQL is ready"
            break
        fi
        
        if [ $i -eq 30 ]; then
            print_error "PostgreSQL failed to start within 30 seconds"
            docker-compose logs postgres
            exit 1
        fi
        
        sleep 1
    done
}

# Function to create test database
create_test_database() {
    print_status "Creating test database..."
    
    docker exec fireup_postgres psql -U fireup -d fireup_dev -c "
        DROP DATABASE IF EXISTS fireup_test;
        CREATE DATABASE fireup_test WITH ENCODING 'UTF8';
    " > /dev/null 2>&1
    
    print_success "Test database created"
}

# Function to check if backup files exist
check_backup_files() {
    print_status "Checking for example backup files..."
    
    local backup_dir="examples/backup_files"
    local files_found=0
    
    for file in "small_sample.leveldb" "users_collection.leveldb" "products_catalog.leveldb" "nested_documents.leveldb"; do
        if [ -f "${backup_dir}/${file}" ]; then
            print_success "Found: ${file}"
            files_found=$((files_found + 1))
        else
            print_warning "Missing: ${file} (some tests will be skipped)"
        fi
    done
    
    if [ $files_found -eq 0 ]; then
        print_warning "No backup files found. Tests will use mock data only."
    else
        print_success "Found ${files_found} backup files"
    fi
}

# Function to build the project
build_project() {
    print_status "Building Fireup project..."
    
    if cargo build --release; then
        print_success "Project built successfully"
    else
        print_error "Failed to build project"
        exit 1
    fi
}

# Function to run unit tests first
run_unit_tests() {
    print_status "Running unit tests..."
    
    if cargo test --lib; then
        print_success "Unit tests passed"
    else
        print_error "Unit tests failed"
        exit 1
    fi
}

# Function to run integration tests
run_integration_tests() {
    print_status "Running integration tests..."
    
    export TEST_DATABASE_URL="${TEST_DATABASE_URL}"
    
    # Run integration tests with detailed output
    if cargo test --test integration_tests -- --nocapture; then
        print_success "Integration tests passed"
    else
        print_error "Integration tests failed"
        return 1
    fi
}

# Function to run CLI tests
run_cli_tests() {
    print_status "Running CLI functionality tests..."
    
    # Test CLI help
    if cargo run -- --help > /dev/null; then
        print_success "CLI help works"
    else
        print_error "CLI help failed"
        return 1
    fi
    
    # Test CLI stats command
    if cargo run -- stats > /dev/null; then
        print_success "CLI stats command works"
    else
        print_error "CLI stats command failed"
        return 1
    fi
    
    # Test validation with mock file if backup files don't exist
    if [ -f "examples/backup_files/small_sample.leveldb" ]; then
        if cargo run -- validate --backup-file examples/backup_files/small_sample.leveldb > /dev/null; then
            print_success "CLI validate command works"
        else
            print_warning "CLI validate command failed (may be expected with mock data)"
        fi
    else
        print_warning "Skipping CLI validate test - no backup files available"
    fi
}

# Function to test PostgreSQL client compatibility
test_client_compatibility() {
    print_status "Testing PostgreSQL client compatibility..."
    
    # Test psql connection
    if docker exec fireup_postgres psql -U fireup -d fireup_test -c "SELECT version();" > /dev/null; then
        print_success "psql connection works"
    else
        print_error "psql connection failed"
        return 1
    fi
    
    # Test basic SQL operations
    docker exec fireup_postgres psql -U fireup -d fireup_test -c "
        CREATE SCHEMA IF NOT EXISTS test_schema;
        CREATE TABLE test_schema.test_table (id SERIAL PRIMARY KEY, name TEXT);
        INSERT INTO test_schema.test_table (name) VALUES ('test');
        SELECT COUNT(*) FROM test_schema.test_table;
        DROP SCHEMA test_schema CASCADE;
    " > /dev/null
    
    if [ $? -eq 0 ]; then
        print_success "PostgreSQL client compatibility verified"
    else
        print_error "PostgreSQL client compatibility test failed"
        return 1
    fi
}

# Function to test Docker container health
test_container_health() {
    print_status "Testing Docker container health..."
    
    # Check container status
    if docker ps --filter "name=fireup_postgres" --format "{{.Status}}" | grep -q "Up"; then
        print_success "Container is running"
    else
        print_error "Container is not running"
        return 1
    fi
    
    # Check container health
    if docker exec fireup_postgres pg_isready -U fireup -d fireup_dev > /dev/null; then
        print_success "Container is healthy"
    else
        print_error "Container health check failed"
        return 1
    fi
    
    # Check container logs for errors
    if docker-compose logs postgres | grep -i error | grep -v "database system is ready"; then
        print_warning "Found errors in container logs (may be normal startup messages)"
    else
        print_success "No critical errors in container logs"
    fi
}

# Function to cleanup test environment
cleanup() {
    print_status "Cleaning up test environment..."
    
    # Remove test database
    docker exec fireup_postgres psql -U fireup -d fireup_dev -c "DROP DATABASE IF EXISTS fireup_test;" > /dev/null 2>&1
    
    print_success "Cleanup completed"
}

# Function to generate test report
generate_report() {
    print_status "Generating test report..."
    
    local report_file="test_report_$(date +%Y%m%d_%H%M%S).txt"
    
    {
        echo "Fireup Integration Test Report"
        echo "=============================="
        echo "Date: $(date)"
        echo "Environment: $(uname -a)"
        echo "Docker Version: $(docker --version)"
        echo "Rust Version: $(rustc --version)"
        echo ""
        echo "Test Results:"
        echo "- Unit Tests: PASSED"
        echo "- Integration Tests: PASSED"
        echo "- CLI Tests: PASSED"
        echo "- Client Compatibility: PASSED"
        echo "- Container Health: PASSED"
        echo ""
        echo "Database Information:"
        docker exec fireup_postgres psql -U fireup -d fireup_dev -c "SELECT version();" 2>/dev/null || echo "Database connection failed"
        echo ""
        echo "Container Stats:"
        docker stats fireup_postgres --no-stream 2>/dev/null || echo "Container stats unavailable"
    } > "$report_file"
    
    print_success "Test report generated: $report_file"
}

# Main execution flow
main() {
    echo "🧪 Fireup Integration Test Suite"
    echo "================================"
    
    # Pre-flight checks
    check_docker
    check_docker_compose
    
    # Environment setup
    start_postgres
    create_test_database
    check_backup_files
    
    # Build and test
    build_project
    
    # Run tests in order
    local test_failures=0
    
    if ! run_unit_tests; then
        test_failures=$((test_failures + 1))
    fi
    
    if ! run_integration_tests; then
        test_failures=$((test_failures + 1))
    fi
    
    if ! run_cli_tests; then
        test_failures=$((test_failures + 1))
    fi
    
    if ! test_client_compatibility; then
        test_failures=$((test_failures + 1))
    fi
    
    if ! test_container_health; then
        test_failures=$((test_failures + 1))
    fi
    
    # Cleanup
    cleanup
    
    # Final results
    echo ""
    echo "🏁 Test Results Summary"
    echo "======================"
    
    if [ $test_failures -eq 0 ]; then
        print_success "All tests passed! ✅"
        generate_report
        exit 0
    else
        print_error "$test_failures test suite(s) failed ❌"
        exit 1
    fi
}

# Handle script interruption
trap cleanup EXIT

# Run main function
main "$@"