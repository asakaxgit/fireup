# GitHub Copilot Instructions for Fireup

## Project Overview

Fireup is a migration tool that provides dual compatibility with both PostgreSQL and Firestore, enabling seamless migration of Firestore backup data to PostgreSQL while supporting Standard SQL queries for data access and manipulation.

**Key Capabilities:**
- Parse Firestore LevelDB backup files
- Automatic schema analysis and normalization
- Direct PostgreSQL import with optimized schema
- Standard SQL query support
- Comprehensive logging and error handling

## Technology Stack

- **Language**: Rust 1.70+
- **Async Runtime**: Tokio
- **Database**: PostgreSQL 15+ (via tokio-postgres, deadpool-postgres)
- **Serialization**: serde, serde_json
- **CLI Framework**: clap 4.0
- **Logging**: tracing, tracing-subscriber
- **Error Handling**: anyhow, thiserror
- **Testing**: cargo test, tokio-test
- **Container**: Docker, Docker Compose

## Architecture

The codebase is organized into five main modules:

1. **leveldb_parser**: Parses Firestore LevelDB backup files
   - `parser.rs`: Main parsing logic
   - `validator.rs`: Backup validation logic

2. **schema_analyzer**: Analyzes and normalizes Firestore schemas
   - `analyzer.rs`: Document structure analysis
   - `normalizer.rs`: Database normalization
   - `ddl_generator.rs`: DDL generation for PostgreSQL
   - `constraint_analyzer.rs`: Constraint analysis
   - `index_generator.rs`: Index generation

3. **data_importer**: Handles PostgreSQL import
   - `importer.rs`: Main import logic
   - `transformer.rs`: Data transformation
   - `type_mapper.rs`: Type mapping between Firestore and PostgreSQL
   - `sql_generator.rs`: SQL generation

4. **error**: Centralized error handling
   - Custom error types with context using thiserror
   - Structured error messages with suggestions

5. **monitoring**: Logging and observability
   - Structured logging with tracing
   - Performance metrics

## Development Setup

### Prerequisites
- Rust 1.70+ installed (`rustup` recommended)
- Docker and Docker Compose for PostgreSQL
- Git for version control

### Initial Setup
```bash
# Clone the repository
git clone https://github.com/asakaxgit/fireup.git
cd fireup

# Copy environment configuration
cp .env.example .env

# Start PostgreSQL
docker-compose up -d postgres

# Build the project
cargo build

# Run tests
cargo test
```

## Building and Testing

### Build Commands
```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Check without building
cargo check
```

### Testing Commands
```bash
# Run all tests
cargo test

# Run with output visible
cargo test -- --nocapture

# Run specific test module
cargo test leveldb_parser

# Run integration tests only
cargo test --test integration_tests

# Run deployment tests
cargo test --test deployment_tests
```

### Linting and Formatting
```bash
# Check code formatting
cargo fmt -- --check

# Format code
cargo fmt

# Run Clippy (linter)
cargo clippy -- -D warnings

# Run Clippy with fixes
cargo clippy --fix
```

## Code Style and Conventions

### Rust Style Guidelines
- Follow standard Rust formatting conventions (use `cargo fmt`)
- Enable all Clippy warnings in CI (`-D warnings`)
- Use descriptive variable names
- Prefer explicit error handling over unwrap/expect
- Document public APIs with doc comments (`///`)

### Error Handling
- Use the custom `FireupError` enum for domain errors
- Provide contextual error messages with suggestions
- Include error context (file paths, collection names, field paths)
- Use `thiserror` for error definitions
- Use `anyhow` for error propagation in non-library code

Example:
```rust
FireupError::DocumentParse {
    message: "Invalid field type".to_string(),
    document_path: Some(doc_path),
    context: ErrorContext::new(),
    suggestions: vec!["Check field type mapping".to_string()],
}
```

### Logging
- Use structured logging with the `tracing` crate
- Log levels:
  - `error!`: Critical failures
  - `warn!`: Recoverable issues
  - `info!`: Important state changes
  - `debug!`: Detailed debugging info
  - `trace!`: Very verbose debugging
- Include contextual fields in log messages
- Set `RUST_LOG` environment variable for log filtering

Example:
```rust
use tracing::{info, warn, error};

info!(collection = %collection_name, "Processing collection");
warn!(document_id = %doc_id, "Missing field in document");
error!(error = %err, "Failed to import data");
```

### Naming Conventions
- **Types/Structs**: PascalCase (e.g., `FirestoreDocument`, `TableDefinition`)
- **Functions/Methods**: snake_case (e.g., `parse_backup`, `analyze_schema`)
- **Constants**: SCREAMING_SNAKE_CASE (e.g., `MAX_BATCH_SIZE`)
- **Module names**: snake_case (e.g., `leveldb_parser`, `schema_analyzer`)

### Async Code
- Use `async/await` for I/O operations
- Use `tokio` runtime features appropriately
- Avoid blocking operations in async contexts
- Use `spawn_blocking` for CPU-intensive work

### Database Interactions
- Use connection pooling (deadpool-postgres)
- Prepare statements for repeated queries
- Use transactions for multi-step operations
- Handle SQL errors gracefully
- Parameterize all SQL queries (no string concatenation)

### Testing Patterns
- Unit tests in the same file as the code (using `#[cfg(test)]`)
- Integration tests in the `tests/` directory
- Use descriptive test names: `test_<what>_<condition>_<expected>`
- Set up test fixtures for database tests
- Clean up resources in test teardown
- Mock external dependencies when appropriate

Example:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_document_with_valid_data_succeeds() {
        // Arrange
        let input = create_test_document();
        
        // Act
        let result = parse_document(input);
        
        // Assert
        assert!(result.is_ok());
    }
}
```

## Common Patterns

### Module Structure
Each major module follows this pattern:
```
module_name/
├── mod.rs          # Module exports
├── main_logic.rs   # Core implementation
├── tests.rs        # Unit tests
└── helper.rs       # Helper utilities
```

### Type Definitions
- Define domain types in `types.rs`
- Use `#[derive(Debug, Clone, Serialize, Deserialize)]` for data types
- Document struct fields with doc comments
- Use enums for fixed sets of values

### Configuration
- Load configuration from environment variables
- Provide sensible defaults (see `.env.example` for default values)
- Use `.env` files for local development (copy from `.env.example`)
- Document all configuration options in `.env.example`

## Security Guidelines

### Input Validation
- Validate all external input (backup files, user input)
- Sanitize file paths to prevent directory traversal
- Validate SQL generated from user data
- Use type-safe APIs where possible

### SQL Injection Prevention
- Always use parameterized queries
- Never concatenate user input into SQL strings
- Use the query builder or prepared statements
- Validate and sanitize table/column names from Firestore

### Secrets Management
- Never commit credentials or secrets
- Use environment variables for sensitive data
- Keep `.env` files out of version control
- Document required environment variables in `.env.example`

### Error Messages
- Don't expose sensitive information in error messages
- Log detailed errors but return sanitized messages to users
- Be careful with stack traces in production

## Common Tasks

### Adding a New CLI Command
1. Add command variant to `Commands` enum in `main.rs`
2. Implement command handler function
3. Add command documentation
4. Add tests for the command
5. Update README with usage examples

### Adding a New PostgreSQL Type Mapping
1. Add type variant to `PostgreSQLType` in `types.rs`
2. Update type mapper in `data_importer/type_mapper.rs`
3. Add DDL generation support in `schema_analyzer/ddl_generator.rs`
4. Add tests for the type conversion

### Modifying Schema Analysis
1. Update analysis logic in `schema_analyzer/analyzer.rs`
2. Update normalization if needed in `schema_analyzer/normalizer.rs`
3. Add/update tests in `schema_analyzer/tests.rs`
4. Verify DDL generation still works

## Troubleshooting Development Issues

### PostgreSQL Connection Issues
- Ensure Docker container is running: `docker-compose ps`
- Check port availability: `netstat -an | grep 5433`
- Verify credentials in `.env` file
- Check PostgreSQL logs: `docker-compose logs postgres`

### Build Failures
- Update dependencies: `cargo update`
- Clean build artifacts: `cargo clean`
- Check Rust version: `rustc --version` (must be 1.70+)

### Test Failures
- Ensure PostgreSQL is running for integration tests
- Check environment variable `TEST_DATABASE_URL`
- Review test logs with `cargo test -- --nocapture`
- Check for test file conflicts or race conditions

## CI/CD

The project uses GitHub Actions for continuous integration:
- **Format Check**: `cargo fmt -- --check`
- **Linting**: `cargo clippy -- -D warnings`
- **Build**: `cargo build --verbose`
- **Tests**: `cargo test --verbose`
- **Docker**: Build and test Docker images

All checks must pass before merging PRs.

## Contributing Guidelines

When contributing code:
1. Create a feature branch from `main` or `develop`
2. Write tests for new functionality
3. Ensure all tests pass locally
4. Run `cargo fmt` and `cargo clippy`
5. Update documentation as needed
6. Create a PR with clear description
7. Address code review feedback

## Performance Considerations

- Use batch operations for database imports
  - Configurable via `FIREUP_MAX_BATCH_SIZE` (default: 1000, see `.env.example`)
- Connection pooling is configured via `FIREUP_CONNECTION_POOL_SIZE` (default: 10, see `.env.example`)
- Import timeout can be set via `FIREUP_IMPORT_TIMEOUT_SECONDS` (default: 3600, see `.env.example`)
- All configuration variables are documented in `.env.example` with defaults
- Use `rayon` for parallel processing where appropriate
- Profile CPU-intensive operations with `cargo flamegraph`
- Monitor memory usage during large imports

## Documentation

- Keep README.md up to date with CLI usage
- Document public APIs with doc comments
- Update DEPLOYMENT.md for operational changes
- Generate docs with `cargo doc --open`

## Resources

- [Rust Book](https://doc.rust-lang.org/book/)
- [Tokio Documentation](https://tokio.rs/)
- [PostgreSQL Documentation](https://www.postgresql.org/docs/)
- [Clap CLI Documentation](https://docs.rs/clap/)
