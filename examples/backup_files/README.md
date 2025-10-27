# Example Firestore Backup Files

This directory contains example Firestore backup files for testing the Fireup migration tool.

## File Structure

```
examples/backup_files/
├── README.md                    # This file
├── small_sample.leveldb         # Small test backup (< 1MB)
├── users_collection.leveldb     # Sample users collection
├── products_catalog.leveldb     # E-commerce product catalog
└── nested_documents.leveldb     # Complex nested document structures
```

## Sample Data Descriptions

### small_sample.leveldb
- **Size**: ~500KB
- **Collections**: users, posts
- **Documents**: 100 users, 250 posts
- **Features**: Basic field types, simple relationships
- **Use Case**: Quick testing and development

### users_collection.leveldb
- **Size**: ~2MB
- **Collections**: users, user_profiles, user_settings
- **Documents**: 1,000 users with profiles and settings
- **Features**: Nested objects, arrays, references
- **Use Case**: Testing user management schema normalization

### products_catalog.leveldb
- **Size**: ~5MB
- **Collections**: products, categories, reviews, orders
- **Documents**: 500 products, 50 categories, 2,000 reviews, 300 orders
- **Features**: Complex relationships, varied data types, large text fields
- **Use Case**: E-commerce schema testing

### nested_documents.leveldb
- **Size**: ~1MB
- **Collections**: organizations, departments, employees
- **Documents**: Deeply nested organizational structure
- **Features**: Multi-level nesting, array of objects, complex hierarchies
- **Use Case**: Testing normalization of complex nested structures

## Usage Examples

### Basic Import Test
```bash
# Test with small sample
cargo run -- import --backup-file examples/backup_files/small_sample.leveldb

# Validate backup integrity
cargo run -- validate --backup-file examples/backup_files/small_sample.leveldb
```

### Schema Analysis
```bash
# Analyze users collection schema
cargo run -- analyze --backup-file examples/backup_files/users_collection.leveldb --output users_schema.sql

# Analyze complex nested structures
cargo run -- analyze --backup-file examples/backup_files/nested_documents.leveldb --output nested_schema.sql
```

### Performance Testing
```bash
# Test with larger dataset
cargo run -- import --backup-file examples/backup_files/products_catalog.leveldb --batch-size 1000
```

## Creating Your Own Test Files

To create test backup files from your Firestore data:

1. **Export from Firebase Console**:
   - Go to Firebase Console → Project Settings → Service Accounts
   - Generate a new private key
   - Use the Firebase Admin SDK to export data

2. **Using Firebase CLI**:
   ```bash
   # Install Firebase CLI
   npm install -g firebase-tools
   
   # Login to Firebase
   firebase login
   
   # Export Firestore data
   firebase firestore:export gs://your-bucket/exports/
   ```

3. **Convert to LevelDB Format**:
   The exported data needs to be in LevelDB format. Use Google Cloud Storage export feature or the Firestore emulator for testing.

## Data Privacy Notice

⚠️ **Important**: These example files contain only synthetic/fake data for testing purposes. Never commit real user data or production backups to version control.

## File Format Notes

These `.leveldb` files follow the LevelDB log format specification used by Firestore exports:

- **Magic Number**: Each file starts with LevelDB magic bytes
- **Block Structure**: Data is organized in blocks with CRC32 checksums
- **Compression**: Blocks may be compressed using Snappy compression
- **Key Format**: Keys follow Firestore's internal key encoding scheme
- **Value Format**: Values are Protocol Buffer encoded Firestore documents

For more details on the format, see the [LevelDB documentation](https://github.com/google/leveldb/blob/main/doc/log_format.md).