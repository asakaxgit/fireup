# Fireup - Firestore to PostgreSQL Migration Tool

Fireup is a migration tool that provides dual compatibility with both PostgreSQL and Firestore, enabling seamless migration of Firestore backup data to PostgreSQL while supporting Standard SQL queries for data access and manipulation.

## Features

- Parse Firestore LevelDB backup files
- Automatic schema analysis and normalization
- Direct PostgreSQL import with optimized schema
- Standard SQL query support
- Comprehensive logging and error handling
- Docker-based development environment

## Quick Start

### Prerequisites

- Rust 1.70+ 
- Docker and Docker Compose
- Git

### Development Setup

1. Clone the repository:
```bash
git clone <repository-url>
cd fireup
```

2. Copy environment configuration:
```bash
cp .env.example .env
```

3. Start PostgreSQL backend:
```bash
docker-compose up -d postgres
```

4. Build the project:
```bash
cargo build
```

5. Run the CLI:
```bash
cargo run -- --help
```

### Available Commands

- `fireup import` - Import Firestore backup data to PostgreSQL
- `fireup analyze` - Analyze schema from backup file and generate DDL
- `fireup validate` - Validate backup file integrity

### Example Usage

```bash
# Validate a backup file
cargo run -- validate --backup-file /path/to/backup.leveldb

# Analyze schema and generate DDL
cargo run -- analyze --backup-file /path/to/backup.leveldb --output schema.sql

# Import data to PostgreSQL
cargo run -- import --backup-file /path/to/backup.leveldb --postgres-url postgresql://fireup:fireup_dev_password@localhost:5432/fireup_dev
```

## PostgreSQL Client Connection

Once data is imported, you can connect to PostgreSQL using any standard client:

### Using psql

```bash
# Connect to the development database
psql -h localhost -p 5433 -U fireup -d fireup_dev

# List imported tables
\dt fireup_data.*

# Query imported data
SELECT * FROM fireup_data.import_summary;
```

### Using pgAdmin

1. Create a new server connection:
   - Host: localhost
   - Port: 5433
   - Database: fireup_dev
   - Username: fireup
   - Password: fireup_dev_password

2. Navigate to `fireup_dev` → `Schemas` → `fireup_data` to see imported tables

### Connection String Format

```
postgresql://fireup:fireup_dev_password@localhost:5433/fireup_dev
```

## Development

### Project Structure

```
src/
├── main.rs              # CLI entry point
├── error.rs             # Error handling
├── types.rs             # Core data structures
├── monitoring.rs        # Logging and monitoring
├── leveldb_parser/      # LevelDB parsing functionality
│   ├── parser.rs        # Main parsing logic
│   └── validator.rs     # Backup validation
├── schema_analyzer/     # Schema analysis and normalization
│   ├── analyzer.rs      # Document structure analysis
│   ├── normalizer.rs    # Database normalization
│   ├── ddl_generator.rs # DDL generation
│   └── ...
└── data_importer/       # PostgreSQL import functionality
    ├── importer.rs      # Main import logic
    ├── transformer.rs   # Data transformation
    └── ...
```

### Environment Variables

Copy `.env.example` to `.env` and customize:

```bash
# PostgreSQL Configuration
POSTGRES_HOST=localhost
POSTGRES_PORT=5433
POSTGRES_DB=fireup_dev
POSTGRES_USER=fireup
POSTGRES_PASSWORD=fireup_dev_password

# Application Configuration
FIREUP_MAX_BATCH_SIZE=1000
FIREUP_CONNECTION_POOL_SIZE=10
RUST_LOG=fireup=debug,info
```

### Running Tests

```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Run specific test module
cargo test leveldb_parser
```

### Docker Commands

```bash
# Start PostgreSQL
docker-compose up -d postgres

# View logs
docker-compose logs -f postgres

# Stop services
docker-compose down

# Reset database (removes all data)
docker-compose down -v
docker-compose up -d postgres
```

### Logging

The application uses structured logging with the `tracing` crate:

- Set `RUST_LOG=debug` for verbose output
- Set `RUST_LOG=fireup=info` for application-specific logs
- Logs include structured fields for filtering and analysis

## Troubleshooting

### Common Issues

1. **PostgreSQL connection refused**
   - Ensure Docker container is running: `docker-compose ps`
   - Check port availability: `netstat -an | grep 5433`

2. **Permission denied errors**
   - Verify PostgreSQL user permissions in `init.sql`
   - Check Docker volume permissions

3. **Out of memory during import**
   - Reduce `FIREUP_MAX_BATCH_SIZE` in environment
   - Increase Docker memory limits in `docker-compose.yml`

## License

MIT License