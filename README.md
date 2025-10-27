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

## Development

### Project Structure

```
src/
├── main.rs              # CLI entry point
├── error.rs             # Error handling
├── leveldb_parser/      # LevelDB parsing functionality
├── schema_analyzer/     # Schema analysis and normalization
└── data_importer/       # PostgreSQL import functionality
```

### Running Tests

```bash
cargo test
```

### Logging

The application uses structured logging with the `tracing` crate. Set `RUST_LOG=debug` for verbose output.

## License

MIT License