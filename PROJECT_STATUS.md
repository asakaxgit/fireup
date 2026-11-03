# Fireup Project Status

**Last Updated:** November 2, 2026  
**Version:** 0.1.0  
**Status:** ⛔ NOT production ready — fails against a realistic Firestore/Datastore export

---

## Executive Summary

Fireup aims to migrate Firestore/Datastore backup data into PostgreSQL. The
internal components are built out and unit-tested, but **the end-to-end path
does not work on a realistic managed export.** Validated on 2026-11-02 against
spec-correct export fixtures (SAK-139): `fireup validate` reports
`No records found in LevelDB file`, and `fireup analyze` produces an empty
schema while exiting 0. Zero of 12 entities were recovered.

Treat the previous "Development Complete - Production Ready" assessment as
withdrawn. It was based on unit tests over synthetic in-memory inputs, none of
which exercised the real on-disk export format.

### What is built
- LevelDB log framing reader, schema analyzer with 1NF/2NF/3NF normalization,
  DDL generator, transformer, PostgreSQL importer, CLI, monitoring
- 159 passing library unit tests
- Realistic export fixtures plus a format-level validation suite
  (`tests/fixtures/`, `tests/real_export_validation.rs`)

### What does not work yet
- Reading any real export file (checksum algorithm is wrong — see Residual Risks)
- Decoding real export payloads (wrong protobuf schema)
- Reading more than one kind of an export tree
- Inserting any rows during `import`

---

## Implementation Status

### Core Components

#### 1. LevelDB Parser ⛔ **BLOCKING DEFECTS**
**Status:** Structure in place; does not read real export files  
**Location:** `src/leveldb_parser/`

**Implemented:**
- 32 KiB block iteration and 7-byte record header parsing
- FIRST/MIDDLE/LAST fragment reassembly
- JSON Lines fallback path
- Datastore `v1.Entity` protobuf decoding (`protobuf_entity.rs`)

**Blocking defects (evidence: SAK-139):**
- Checksums are computed with unmasked IEEE CRC-32 instead of LevelDB's masked
  CRC32C, so **every** record of a real export fails validation and is skipped
- Payloads of a real export are legacy `storage_onestore_v3.EntityProto`, not
  `google.datastore.v1.Entity`; nothing decodes
- An export directory is collapsed to a single `output-0`, dropping all other
  kinds and output shards
- `parse_documents()` panics unless `initialize_monitoring()` was called first

**Test Coverage:** unit tests pass but use hand-built inputs that match the
implementation rather than the format. `tests/real_export_validation.rs` adds
format-level coverage.

#### 2. Schema Analyzer ✅ **COMPLETE**
**Status:** Fully implemented with advanced features  
**Location:** `src/schema_analyzer/`

**Capabilities:**
- Automatic document structure analysis
- Field type detection and cataloging
- Nested structure and relationship identification
- Database normalization (1NF, 2NF, 3NF):
  - Eliminates repeating groups (arrays → separate tables)
  - Removes partial dependencies
  - Eliminates transitive dependencies
- Type conflict detection and resolution with warnings
- NOT NULL constraint recommendations based on data completeness
- Comprehensive transformation reporting

**Test Coverage:** Unit tests passing

#### 3. DDL Generator ✅ **COMPLETE**
**Status:** Fully functional with PostgreSQL compatibility  
**Location:** `src/schema_analyzer/ddl_generator.rs`

**Capabilities:**
- Generate CREATE TABLE statements from normalized schemas
- Primary key and foreign key definitions
- Index recommendations for common query patterns
- Constraint generation (NOT NULL, UNIQUE, CHECK)
- Output DDL files for administrator review
- Detailed transformation reports

**Test Coverage:** Unit tests passing

#### 4. Data Transformation ✅ **COMPLETE**
**Status:** Production ready  
**Location:** `src/data_importer/transformer.rs`, `type_mapper.rs`

**Capabilities:**
- Type mapping from Firestore to PostgreSQL:
  - String → VARCHAR/TEXT
  - Number → NUMERIC/INTEGER
  - Boolean → BOOLEAN
  - Timestamp → TIMESTAMP
  - Reference → UUID (foreign keys)
  - Array → Normalized tables
  - Map → JSONB or normalized tables
- Document to relational row conversion
- Foreign key relationship establishment
- Parameterized SQL generation for safe data insertion

**Test Coverage:** Unit tests passing

#### 5. Data Importer ⛔ **NOT WIRED UP**
**Status:** Library methods exist; the CLI never calls them  
**Location:** `src/data_importer/importer.rs`

**Blocking defects (evidence: SAK-139):**
- `src/main.rs` calls only `create_schema`. `import_transformed_data` and
  `execute_full_import` are never invoked, and the pipeline then returns
  `total_records_imported: documents.len()` with `table_imports: vec![]` — so a
  successful-looking `import` writes **zero rows** while reporting the full
  document count as imported
- `parse_postgres_url` discards the `--postgres-url` value entirely and returns
  a hardcoded `postgres:password@localhost:5432/fireup_test`, so the tool cannot
  connect to any other database (and embeds a credential in source)

**Capabilities:**
- Connection pooling with `deadpool-postgres`
- Batch processing for large datasets
- Transaction-based imports with rollback capability
- Progress tracking and resumable imports
- Constraint validation during import
- Configurable batch sizes and connection limits
- Integration with PostgreSQL Docker container

**Test Coverage:** Unit tests and integration tests passing

#### 6. CLI Interface ✅ **COMPLETE**
**Status:** Fully functional with comprehensive command set  
**Location:** `src/main.rs`

**Available Commands:**
```bash
fireup import    # Import Firestore backup to PostgreSQL
fireup analyze   # Analyze schema and generate DDL
fireup validate  # Validate backup file integrity
```

**Features:**
- Configurable logging levels (error, warn, info, debug, trace)
- JSON log output support
- Customizable batch sizes and connection pools
- Schema normalization options
- Detailed progress reporting
- Comprehensive help documentation

**Test Coverage:** Integration tests available

#### 7. Monitoring & Logging ✅ **COMPLETE**
**Status:** Production ready  
**Location:** `src/monitoring.rs`

**Capabilities:**
- Structured logging with `tracing` crate
- Performance metric tracking
- Audit logging for data operations
- Operation timing and statistics
- Error tracking with full context
- Configurable log levels and formats

**Test Coverage:** Unit tests passing

---

## Build & Test Status

### Build Status ✅
```
Compiler: rustc (Rust 1.70+)
Build Time: ~35 seconds
Status: SUCCESS
Warnings: 110 warnings (mostly unused code for future features)
```

**Notes on Warnings:**
- Most warnings are for unused struct fields and methods that are part of the designed API but not yet utilized
- No errors or critical warnings
- All warnings are related to dead code analysis of derived traits
- These represent planned features and comprehensive API surface

### Test Status ⚠️
```
Library unit tests:        159 passed, 0 failed       (cargo test --lib)
Real-export validation:     3 passed, 4 ignored       (cargo test --test real_export_validation)
Test Time: <1 second
Coverage: components covered; end-to-end format path NOT covered
```

The 4 ignored tests are pending acceptance tests that assert correct behaviour
against the export fixtures. They fail today — run them with
`cargo test --test real_export_validation -- --ignored` to see the current gaps.
The high library test count does not indicate format correctness: those tests
build inputs that match the implementation, so they pass while the tool cannot
read a real export.

**Test Categories:**
- Unit tests for LevelDB parsing
- Schema analysis and normalization tests
- DDL generation tests
- Data transformation tests
- PostgreSQL import tests
- Integration tests available
- Deployment tests available

### Integration Testing ⛔
**Location:** `tests/`

`cargo test` does not compile — `tests/integration_tests.rs` is stale against
`src/types.rs` (residual risk 12). This is pre-existing, not a regression. The
suites below exist as files; only `real_export_validation` currently builds and
runs.

**Test Suites:**
- `real_export_validation.rs` - Real-export format validation (builds, runs)
- `integration_tests.rs` - End-to-end workflow testing (does not compile)
- `deployment_tests.rs` - Docker and PostgreSQL testing
- `functional-tests/` - Feature-specific tests
- `data-generator/` - Test data generation utilities

---

## Dependencies & Environment

### Runtime Dependencies
- **PostgreSQL:** 15+ (via Docker)
- **Docker:** 28.0.4+ ✅ Available
- **Docker Compose:** v2.38.2+ ✅ Available
- **Rust:** 1.70+ ✅ Required for building

### Key Rust Dependencies
```toml
tokio = "1.0"                    # Async runtime
tokio-postgres = "0.7"           # PostgreSQL client
deadpool-postgres = "0.12"       # Connection pooling
serde = "1.0"                    # Serialization
serde_json = "1.0"               # JSON handling
clap = "4.0"                     # CLI parsing
tracing = "0.1"                  # Structured logging
anyhow = "1.0"                   # Error handling
chrono = "0.4"                   # Date/time handling
```

### Development Environment
- Docker Compose configuration ready
- PostgreSQL initialization scripts available (`init.sql`)
- Environment variable templates (`.env.example`)
- Comprehensive documentation

---

## Documentation Status

### Available Documentation ✅

#### User Documentation
- **README.md** - Comprehensive project overview, quick start, and usage examples
- **DEPLOYMENT.md** - Detailed deployment guide for development and production
- **docs/INTEGRATION_TESTING.md** - Integration testing procedures
- **tests/DEPLOYMENT_TESTING.md** - Deployment testing documentation

#### Developer Documentation
- **.kiro/specs/firestore-postgres-migration/requirements.md** - Complete requirements
- **.kiro/specs/firestore-postgres-migration/design.md** - Architecture and design
- **.kiro/specs/firestore-postgres-migration/tasks.md** - Implementation task tracking
- Inline code documentation throughout source files

#### Operational Documentation
- Docker Compose configuration with comments
- Environment variable documentation
- PostgreSQL client connection guides
- Troubleshooting sections in README and DEPLOYMENT.md

**Documentation Quality:** Excellent - All major aspects covered

---

## Features Implemented vs. Planned

### Requirements Compliance

#### ⛔ Requirement 1: Firestore Backup Import
- [ ] Parse LevelDB backup files per Google's specification — framing reader
      exists but rejects every real record (residual risk 1)
- [ ] Validate backup integrity with CRC32 checksums — wrong algorithm; LevelDB
      uses masked CRC32C (residual risk 1)
- [x] Provide detailed error messages for corrupted files
- [ ] Import parsed data to PostgreSQL-compatible format — schema is created,
      no rows are written (residual risk 3)
- [ ] Generate summary reports of import operations — the summary reports counts
      that were never imported (residual risk 3)

#### ✅ Requirement 2: Schema Analysis & Normalization
- [x] Automatic document structure detection
- [x] Field type and nested structure identification
- [x] Automatic normalization (1NF, 2NF, 3NF)
- [x] Type conflict detection with warnings
- [x] NOT NULL constraint recommendations
- [x] DDL file generation for review
- [x] Detailed transformation reporting

#### ⚠️ Requirement 3: SQL Query Engine (Not Implemented - By Design)
- [N/A] Standard SQL SELECT queries
- [N/A] Query translation for Firestore data
- [N/A] SQL clause support (WHERE, ORDER BY, etc.)

**Note:** This requirement was intentionally replaced with direct PostgreSQL integration. Instead of building a custom query engine, Fireup imports data directly into PostgreSQL, allowing users to leverage PostgreSQL's native query capabilities.

#### ✅ Requirement 4: PostgreSQL Compatibility
- [x] PostgreSQL Docker backend integration
- [x] Standard PostgreSQL authentication (via Docker container)
- [x] Standard PostgreSQL table and view presentation
- [x] PostgreSQL metadata compatibility
- [x] Connection pooling and session management

#### ✅ Requirement 5: Logging & Monitoring
- [x] Comprehensive operation logging with timestamps
- [x] Query performance metrics tracking
- [x] Detailed error logging with context
- [x] Progress indicators for long operations
- [x] Audit logs for data operations

---

## Residual Risks

**Evidence basis.** All items below were observed on 2026-11-02 under SAK-139 by
running the CLI against `tests/fixtures/export-onestore/` and
`tests/fixtures/export-v1-entity/`. Those fixtures are independently verified as
spec-correct by `tests/fixtures/verify_datastore_export.py`, so these are tool
defects, not fixture artifacts. Severity is "does this silently lose data?".

### Critical — silent data loss

1. **Wrong record checksum algorithm** (`parser.rs:360` `calculate_crc32`)
   Uses `crc32fast` (unmasked IEEE CRC-32) over `type || data`. LevelDB uses
   **masked CRC32C**: Castagnoli `0x82F63B78`, masked with
   `((crc >> 15) | (crc << 17)) + 0xA282EAD8`. Every record of a real export
   therefore mismatches, `parse_record_at_offset` errors, and
   `parse_block_records` advances one byte at a time until the block is
   exhausted. Observed: `validate` → `No records found in LevelDB file`;
   0 of 12 entities read. Re-framing the same fixtures with the bug-compatible
   checksum recovers all records, which isolates this as the sole framing defect.

2. **Wrong payload protobuf schema** (`protobuf_entity.rs`)
   Decodes `google.datastore.v1.Entity` (the API proto). A managed export
   contains legacy `storage_onestore_v3.EntityProto`, with unrelated field
   numbers, **proto2 groups** (which `prost` cannot represent at all), and types
   carried in `Property.meaning` rather than in the value field. Observed: with
   framing bypassed, 6 of 6 records read and 0 documents decoded. Field-level
   notes are in `tests/fixtures/README.md`.

3. **`import` reports rows it never wrote** (`main.rs:398-407`)
   Only `create_schema` is called; the result is then constructed with
   `total_records_imported: documents.len()`. A run that creates tables and
   inserts nothing reports a full successful import. This is the most dangerous
   defect in the tool: it would be read as a completed migration.

4. **Document identity is discarded**
   Generated tables use `id UUID` populated from `uuid::Uuid::new_v4()`, and no
   column holds the Datastore key name or id. Datastore keys are strings or
   `int64` (the fixture includes `ada`, `unicode-ユーザー`, and
   `4503599627370496`). Even a working import would produce unjoinable rows.

### High — partial data loss or failed migrations

5. **Only one file of an export tree is read** (`parser.rs:92`
   `resolve_backup_path`) An export is split per kind and per `output-N` shard.
   The resolver returns the first `output-0` found, so all other kinds and shards
   are dropped with no warning. Observed: the 12-entity tree yields at most one
   kind.

6. **Generated DDL is not always loadable**
   `Orders` DDL emits `user TEXT NOT NULL` unquoted; `user` is reserved, and
   `psql` rejects it with `syntax error at or near "user"`. Identifiers are never
   quoted, so table names also silently fold to lower case.

7. **`NOT NULL` emitted for sparse fields**
   `nickname` is null in 4 of 5 fixture users yet is generated `TEXT NOT NULL`.
   Any real import would violate the constraint.

8. **Type mapping loses fidelity**
   Observed in generated DDL: timestamp → `TEXT`, blob → `JSONB` (should be
   `BYTEA`), `int64` → `INTEGER` (overflows on auto-allocated key ids),
   nested map emitted both as `address JSONB` *and* flattened
   `address_city`/`address_postcode`/`address_verified` (duplicated data).

### Medium — operability

9. **Failures are reported as success.** `analyze` exits 0 after parsing 0
   documents and generating 0 tables; `validate --check-quality` reports
   `Integrity score: 1.00` and `is_valid` when 0 documents decoded from valid
   records. A migration tool must exit non-zero on total parse loss.

10. **Hardcoded credentials / ignored connection string.** `parse_postgres_url`
    returns a fixed `postgres:password@localhost:5432/fireup_test` regardless of
    `--postgres-url`.

11. **Library API panics.** `parse_documents()` panics via
    `get_monitoring_system()` unless `initialize_monitoring()` ran first.

12. **`cargo test` does not compile.** `tests/integration_tests.rs` fails with 9
    errors (`parse_backup` called without the `LevelDBParser` trait in scope;
    `ColumnDefinition.data_type`, renamed to `column_type`). Pre-existing and
    long-standing — reproduced at `fd24bf2`, before any SAK-137/SAK-139 work.
    Only `cargo test --lib` and the individual working test targets run, so the
    "integration tests available / passing" claim in this document was never
    true. Either fix or remove the stale targets so `cargo test` is meaningful.

### Not yet assessed

- **`import` end-to-end.** Could not be executed: the hardcoded connection
  target is unreachable in the validation environment. Findings 3 and 10 are
  from source inspection; everything else above was observed at runtime.
- **Performance / large exports.** No benchmark has been run against a
  multi-shard export.
- **A genuine GCP-produced export.** The fixtures are built to the documented
  format, not captured from a live project. Confirming against a real
  `gcloud firestore export` remains outstanding and should gate any
  production claim.

### Non-issues

1. **Unused API Surface (Not Issues - Design Choice)**
   - Many structs and methods are implemented but not yet utilized
   - Represents comprehensive API design for future features
   - Examples: Advanced SQL generation options, custom type mappings
   - These generate compiler warnings but are intentionally preserved

3. **Test Data Availability**
   - Realistic export fixtures now live in `tests/fixtures/` (see its README),
     generated to the documented format and independently CRC-verified
   - Still missing: a capture from a live `gcloud firestore export`, which is
     what would let any production claim be made

4. **Performance Tuning**
   - Default batch sizes may need adjustment for very large datasets
   - Connection pool sizes are conservative defaults
   - PostgreSQL configuration should be tuned for specific workloads

### Compiler Warnings (110 total)

**Categories:**
- Dead code (unused struct fields) - 45%
- Unused methods - 40%
- Unused types - 15%

**Assessment:** Non-critical. These warnings represent:
- Comprehensive API design with unused features
- Future extensibility points
- Derived trait implementations on unused types

**Recommendation:** Can be addressed in future releases as features are utilized

---

## Production Readiness Assessment

### ⛔ NOT Ready for Production Use

Do not run this against data anyone depends on. It cannot read a real export,
and the `import` path reports success without writing rows — a failure mode that
looks exactly like a completed migration.

**Strengths:**
1. **Clean Architecture:** Well-organized, modular codebase
2. **Unit Test Breadth:** 159 passing library tests
3. **Good Documentation:** Complete user and developer guides
4. **Modern Stack:** Rust + PostgreSQL + Docker
5. **Error Handling:** Comprehensive error types and context
6. **Monitoring:** Built-in logging and metrics

**Exit criteria before any production claim:**

1. Residual risks 1-5 fixed, with the `#[ignore]` removed from the pending
   acceptance tests in `tests/real_export_validation.rs`
2. `validate`, `analyze`, and `import` all pass against
   `tests/fixtures/export-onestore/`, recovering all 12 entities across both
   kinds, with row counts asserted in PostgreSQL after `import`
3. The same three commands pass against an export captured from a live GCP
   project
4. Non-zero exit on total parse loss (residual risk 9)

**Also recommended:**

1. **Test with Real Data** (Priority: HIGH)
   - Capture an export from a live Firestore/Datastore project
   - Test various backup sizes (small, medium, large) and multi-shard exports
   - Verify schema detection accuracy

2. **Performance Validation** (Priority: MEDIUM)
   - Benchmark import performance with large datasets
   - Tune batch sizes and connection pools
   - Optimize PostgreSQL configuration

3. **Security Review** (Priority: HIGH)
   - Review SQL generation for injection vulnerabilities
   - Validate credential management
   - Audit error messages for information disclosure

4. **Code Cleanup** (Priority: LOW)
   - Address unused code warnings
   - Add `#[allow(dead_code)]` attributes where appropriate
   - Document API surface for future features

5. **Monitoring Setup** (Priority: MEDIUM)
   - Configure production logging levels
   - Set up log aggregation
   - Establish alerting for errors

---

## Deployment Status

### Development Environment ✅
- Docker Compose configuration ready
- PostgreSQL container setup complete
- Environment variables documented
- Quick start guide available

### Production Environment ⚠️ NEEDS SETUP
- Production Docker Compose template available
- Security configurations documented
- Backup and recovery procedures documented
- **Action Required:** Create production deployment

### Client Tools Support ✅
**Verified Compatible:**
- psql (PostgreSQL CLI)
- pgAdmin (GUI)
- DBeaver (GUI)
- Python (psycopg2)
- Node.js (pg)
- Any PostgreSQL-compatible tool

---

## Next Steps & Recommendations

### Immediate (Sprint 1)
1. **Real-World Testing**
   - Obtain and test with actual Firestore backup files
   - Validate LevelDB protobuf parsing
   - Document any format variations discovered

2. **Security Audit**
   - Review SQL generation code
   - Test for injection vulnerabilities
   - Validate error handling doesn't leak sensitive data

3. **Performance Baseline**
   - Establish performance benchmarks
   - Document recommended configurations
   - Create performance tuning guide

### Short-Term (Sprint 2-3)
1. **Production Deployment**
   - Set up production environment
   - Configure monitoring and alerting
   - Establish backup procedures

2. **User Acceptance Testing**
   - Work with pilot users
   - Gather feedback on usability
   - Document common issues and solutions

3. **Documentation Enhancement**
   - Add more usage examples
   - Create troubleshooting guides
   - Document common migration patterns

### Long-Term (Future Releases)
1. **Feature Enhancements**
   - Incremental import capabilities
   - Schema migration tracking
   - Custom transformation rules
   - Web-based UI for monitoring

2. **Performance Optimization**
   - Parallel import processing
   - Memory optimization for large files
   - Query performance tuning

3. **Ecosystem Integration**
   - CI/CD pipeline templates
   - Kubernetes deployment manifests
   - Cloud platform integrations (AWS, GCP, Azure)

---

## Code Metrics

### Codebase Statistics
```
Source Files:      25 Rust files
Test Files:        11 Rust files
Total Lines:       ~15,371 lines
Documentation:     8 markdown files
```

### Component Breakdown
```
src/
├── main.rs                    ~400 lines  (CLI interface)
├── error.rs                   ~200 lines  (Error handling)
├── types.rs                   ~400 lines  (Core types)
├── monitoring.rs              ~500 lines  (Logging & metrics)
├── leveldb_parser/            ~3,000 lines (3 files)
├── schema_analyzer/           ~4,500 lines (5 files)
└── data_importer/             ~6,000 lines (4 files)
```

### Test Coverage
- LevelDB Parser: Comprehensive unit tests
- Schema Analyzer: Full normalization testing
- Data Importer: Connection and import tests
- Integration: End-to-end workflow tests
- Deployment: Docker and PostgreSQL tests

---

## Technology Stack

### Core Technologies
- **Language:** Rust 2021 Edition
- **Database:** PostgreSQL 15+
- **Containerization:** Docker & Docker Compose
- **CLI Framework:** Clap 4.0
- **Async Runtime:** Tokio 1.0

### Key Libraries
- **Database:** tokio-postgres, deadpool-postgres
- **Serialization:** serde, serde_json
- **Logging:** tracing, tracing-subscriber
- **Error Handling:** anyhow, thiserror
- **Testing:** tempfile, tokio-test

---

## Conclusion

Fireup is well-organized and broadly unit-tested, but the real-export validation
run under SAK-139 showed the end-to-end migration path does not work: 0 of 12
entities were recovered from a spec-correct export, and `import` would have
reported a full successful migration while writing no rows.

The structure is sound and the gaps are specific and fixable — they are format
correctness bugs (checksum algorithm, payload proto, multi-file handling) plus
unwired plumbing, not architectural problems. The validation harness to prove
each fix now exists in the repo.

**Current State:** Implementation incomplete; blocked on format correctness  
**Recommended Next Step:** Fix residual risks 1-5, driven by the pending
acceptance tests in `tests/real_export_validation.rs`  
**Production Timeline:** Not estimable until a real export round-trips

### Verified
- ✅ 159 library unit tests passing, clean build
- ✅ Comprehensive documentation
- ✅ CLI argument surface functional
- ✅ Realistic, independently verified export fixtures in `tests/fixtures/`
- ✅ `analyze` → DDL → `psql` load path exercised (one of two generated DDL
  files loads; the other is rejected — residual risk 6)

### Outstanding Items
- ⛔ Cannot read a real export (residual risks 1, 2, 5)
- ⛔ `import` writes no rows and misreports success (residual risk 3)
- ⛔ Document identity dropped (residual risk 4)
- ⚠️ Real GCP-captured export still untested
- ⚠️ Performance benchmarking pending
- ⚠️ Production deployment configuration needed
- ⚠️ Security audit recommended (hardcoded credential, residual risk 10)

---

**For questions or issues, please refer to:**
- [README.md](README.md) - General usage and quick start
- [DEPLOYMENT.md](DEPLOYMENT.md) - Deployment procedures
- [Issue Tracker](https://github.com/asakaxgit/fireup/issues) - Report bugs and feature requests
