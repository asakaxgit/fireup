# Functional Tests for Firestore Parser

This directory contains comprehensive functional tests for the Firestore LevelDB parser functionality.

## Test Structure

### Core Test Files

- **`parser_tests.rs`** - Tests for the core parsing functionality
  - JSON document parsing and Firestore value unwrapping
  - Document path parsing and identity extraction
  - Error handling for malformed data
  - Metadata field detection

- **`integration_tests.rs`** - End-to-end integration tests
  - Complete parsing pipeline with sample data
  - Schema analysis and normalization workflow
  - Type inference and conflict resolution
  - Performance testing with large datasets

- **`validation_tests.rs`** - Data validation and quality tests
  - Backup file validation and integrity checks
  - Record type validation and checksum verification
  - Data quality analysis and edge case handling
  - Concurrent access testing

- **`test_utils.rs`** - Shared utilities and test helpers
  - Sample data generation
  - Test document creation
  - Path utilities for test data
  - Common assertion helpers

## Key Test Features

### 1. Parser Functionality Tests
- **JSON Document Parsing**: Tests parsing of Firestore JSON export format with proper value unwrapping
- **Complex Nested Structures**: Validates handling of deeply nested objects and arrays
- **Document Identity Extraction**: Tests extraction of collection names and document IDs from various path formats
- **Firestore Value Types**: Comprehensive testing of all Firestore value types (string, integer, double, boolean, timestamp, array, map)

### 2. Integration Tests
- **End-to-End Pipeline**: Tests complete workflow from LevelDB parsing to schema analysis
- **Schema Analysis**: Validates document structure analysis and field type inference
- **Type Conflict Resolution**: Tests handling of fields with inconsistent types across documents
- **Performance Testing**: Validates performance with datasets of 100+ documents

### 3. Validation Tests
- **File Integrity**: Tests LevelDB file structure validation and checksum verification
- **Record Type Validation**: Validates proper handling of LevelDB record types (Full, First, Middle, Last)
- **Data Quality**: Tests detection of empty documents, null values, and data inconsistencies
- **Error Handling**: Comprehensive error handling tests for various failure scenarios

## Sample Test Data

The tests use sample Firestore export data located in `tests/.firestore-data/`:
- **Firebase Export Format**: Standard Firebase/Firestore export structure
- **LevelDB Binary Data**: Real LevelDB format files for testing binary parsing
- **JSON Documents**: Sample JSON documents with various Firestore value types

## Running the Tests

```bash
# Run all functional tests
cargo test --test functional_tests

# Run specific test modules
cargo test --test functional_tests parser_tests
cargo test --test functional_tests integration_tests
cargo test --test functional_tests validation_tests

# Run with output for debugging
cargo test --test functional_tests -- --nocapture
```

## Test Coverage

### Parser Core Functionality
- ✅ JSON document parsing with Firestore value unwrapping
- ✅ Document path parsing and collection/ID extraction
- ✅ Nested object and array handling
- ✅ Metadata field detection and filtering
- ✅ Error handling for malformed data

### LevelDB Integration
- ✅ File reading and block parsing
- ✅ Record reconstruction from fragments
- ✅ Checksum validation
- ✅ Binary data handling

### Schema Analysis
- ✅ Document structure analysis
- ✅ Field type inference
- ✅ Type conflict detection and resolution
- ✅ Collection grouping and statistics

### Data Quality
- ✅ Empty document handling
- ✅ Null value processing
- ✅ Type consistency validation
- ✅ Large dataset performance

## Expected Test Behavior

### With Sample Data Present
When sample data exists in `tests/.firestore-data/`, the tests will:
- Parse actual LevelDB export files
- Validate real Firestore document structures
- Test with authentic Firebase export formats
- Measure performance with realistic data sizes

### Without Sample Data
When sample data is not available, the tests will:
- Skip file-dependent tests gracefully
- Use mock data for parser logic testing
- Focus on unit-level functionality
- Provide clear skip messages

## Test Data Requirements

For full test coverage, place sample Firestore export data in:
```
tests/.firestore-data/
├── firebase-export-metadata.json
└── firestore_export/
    └── all_namespaces/
        └── all_kinds/
            ├── all_namespaces_all_kinds.export_metadata
            └── output-0
```

## Mock Data Testing

The tests include comprehensive mock data generation for:
- Various Firestore document structures
- Different collection types (users, orders, products, reviews)
- Complex nested objects and arrays
- Type conflicts and edge cases
- Large datasets for performance testing

This ensures the tests can run effectively even without real export data, while still validating all core functionality.