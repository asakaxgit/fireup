# Organized Sample Data Plan

This document outlines the structured approach for creating organized sample data for testing the Firestore to PostgreSQL migration tool. The data progresses from two primitives and two rows and then to more complex data finally, enabling incremental testing and validation.

## Overview

The sample data is organized in progressive levels of complexity, starting with the most basic primitives and gradually introducing more sophisticated data structures. Samples start with tiny dump data based on primitives and one row, then continuously increase in complexity such as two primitives and two rows, progressing to more complex data finally. Each level builds upon the previous one, ensuring comprehensive test coverage.

## Data Structure Hierarchy

### Level 0: Tiny Primitives (Single Field, Single Row)
**Status**: ✅ Implemented (`tiny.ts`)

The simplest possible data - one document with one primitive field.

**Characteristics:**
- Single collection
- Single document
- Single primitive field (string, number, boolean, null, timestamp)
- No nesting, no relationships, no arrays

**Use Cases:**
- Basic parser functionality
- Type detection for primitives
- Minimal schema generation
- Edge case testing (empty collections, null values)

**Example:**
```json
{
  "collection": "tiny_test",
  "documents": [
    {
      "id": "string_doc",
      "data": { "value": "hello" }
    },
    {
      "id": "number_doc",
      "data": { "value": 42 }
    }
  ]
}
```

**Files:**
- Generator: `src/tiny.ts`
- Test: `../functional-tests/tiny_tests.rs`

---

### Level 1: Two Primitives (Two Fields, Single Row)
**Status**: 📋 Planned

Extends Level 0 by adding a second primitive field to the same document.

**Characteristics:**
- Single collection
- Single document
- Two primitive fields of different types
- No nesting, no relationships, no arrays

**Use Cases:**
- Multi-field document parsing
- Type inference with multiple fields
- Column generation for multiple fields
- Field ordering and naming

**Example:**
```json
{
  "collection": "simple_user",
  "documents": [
    {
      "id": "user_001",
      "data": {
        "name": "Alice",
        "age": 30
      }
    }
  ]
}
```

**Planned Files:**
- Generator: `src/level1_two_primitives.ts`
- Test: `../functional-tests/level1_tests.rs`

---

### Level 2: Two Rows (Single Field, Multiple Documents)
**Status**: 📋 Planned

Extends Level 0 by adding a second document with the same structure.

**Characteristics:**
- Single collection
- Two documents
- Same single primitive field in both documents
- No nesting, no relationships, no arrays

**Use Cases:**
- Multiple document parsing
- Collection-level analysis
- Type consistency across documents
- Batch processing

**Example:**
```json
{
  "collection": "simple_items",
  "documents": [
    {
      "id": "item_001",
      "data": { "name": "Apple" }
    },
    {
      "id": "item_002",
      "data": { "name": "Banana" }
    }
  ]
}
```

**Planned Files:**
- Generator: `src/level2_two_rows.ts`
- Test: `../functional-tests/level2_tests.rs`

---

### Level 3: Two Primitives + Two Rows
**Status**: 📋 Planned

Combines Level 1 and Level 2 - multiple documents, each with multiple primitive fields.

**Characteristics:**
- Single collection
- Two documents
- Two primitive fields per document
- No nesting, no relationships, no arrays

**Use Cases:**
- Multi-field, multi-document parsing
- Schema normalization with multiple rows
- Type consistency validation
- Primary key detection

**Example:**
```json
{
  "collection": "users",
  "documents": [
    {
      "id": "user_001",
      "data": {
        "name": "Alice",
        "age": 30
      }
    },
    {
      "id": "user_002",
      "data": {
        "name": "Bob",
        "age": 25
      }
    }
  ]
}
```

**Planned Files:**
- Generator: `src/level3_two_primitives_two_rows.ts`
- Test: `../functional-tests/level3_tests.rs`

---

### Level 4: Arrays (Single Level)
**Status**: 📋 Planned

Introduces array fields containing primitive values.

**Characteristics:**
- Single collection
- Multiple documents
- Arrays of primitives (strings, numbers, booleans)
- No nested objects, no relationships

**Use Cases:**
- Array type handling
- PostgreSQL array column generation
- Array normalization strategies
- Array element type inference

**Example:**
```json
{
  "collection": "products",
  "documents": [
    {
      "id": "product_001",
      "data": {
        "name": "Laptop",
        "tags": ["electronics", "computers", "portable"],
        "prices": [999.99, 899.99, 799.99]
      }
    }
  ]
}
```

**Planned Files:**
- Generator: `src/level4_arrays.ts`
- Test: `../functional-tests/level4_tests.rs`

---

### Level 5: Nested Objects (Single Level)
**Status**: 📋 Planned

Introduces nested object structures.

**Characteristics:**
- Single collection
- Multiple documents
- Nested objects (one level deep)
- No arrays of objects, no relationships

**Use Cases:**
- Nested object parsing
- JSONB column generation
- Flattening strategies
- Nested field path handling

**Example:**
```json
{
  "collection": "users",
  "documents": [
    {
      "id": "user_001",
      "data": {
        "name": "Alice",
        "address": {
          "street": "123 Main St",
          "city": "San Francisco",
          "zip": "94102"
        }
      }
    }
  ]
}
```

**Planned Files:**
- Generator: `src/level5_nested_objects.ts`
- Test: `../functional-tests/level5_tests.rs`

---

### Level 6: Arrays of Objects
**Status**: 📋 Planned

Combines arrays and nested objects.

**Characteristics:**
- Single collection
- Multiple documents
- Arrays containing objects
- Complex nested structures

**Use Cases:**
- Array of objects handling
- Normalization decisions (flatten vs JSONB)
- Complex type inference
- Performance with nested arrays

**Example:**
```json
{
  "collection": "orders",
  "documents": [
    {
      "id": "order_001",
      "data": {
        "customer": "Alice",
        "items": [
          { "product": "Laptop", "quantity": 1, "price": 999.99 },
          { "product": "Mouse", "quantity": 2, "price": 29.99 }
        ]
      }
    }
  ]
}
```

**Planned Files:**
- Generator: `src/level6_arrays_of_objects.ts`
- Test: `../functional-tests/level6_tests.rs`

---

### Level 7: Multiple Collections (Relationships)
**Status**: 📋 Planned

Introduces multiple collections with reference relationships.

**Characteristics:**
- Multiple collections
- Document references between collections
- Foreign key relationships
- One-to-many relationships

**Use Cases:**
- Relationship detection
- Foreign key generation
- Cross-collection normalization
- Referential integrity

**Example:**
```json
{
  "collections": [
    {
      "name": "users",
      "documents": [
        {
          "id": "user_001",
          "data": { "name": "Alice", "email": "alice@example.com" }
        }
      ]
    },
    {
      "name": "orders",
      "documents": [
        {
          "id": "order_001",
          "data": {
            "user_id": "user_001",
            "total": 999.99,
            "status": "completed"
          }
        }
      ]
    }
  ]
}
```

**Planned Files:**
- Generator: `src/level7_relationships.ts`
- Test: `../functional-tests/level7_tests.rs`

---

### Level 8: Subcollections
**Status**: 📋 Planned

Introduces Firestore subcollections (nested collections).

**Characteristics:**
- Parent collections
- Subcollections within documents
- Hierarchical data structure
- Complex path parsing

**Use Cases:**
- Subcollection parsing
- Hierarchical schema generation
- Path-based table naming
- Nested collection relationships

**Example:**
```json
{
  "collection": "users",
  "documents": [
    {
      "id": "user_001",
      "data": { "name": "Alice" },
      "subcollections": [
        {
          "collection": "orders",
          "documents": [
            {
              "id": "order_001",
              "data": { "total": 999.99 }
            }
          ]
        }
      ]
    }
  ]
}
```

**Planned Files:**
- Generator: `src/level8_subcollections.ts`
- Test: `../functional-tests/level8_tests.rs`

---

### Level 9: Complex Real-World Data
**Status**: 📋 Planned

Full-featured, realistic data structure combining all previous levels.

**Characteristics:**
- Multiple collections
- Complex nested structures
- Arrays of objects
- Relationships and references
- Subcollections
- Various data types
- Realistic data volumes

**Use Cases:**
- End-to-end integration testing
- Performance testing
- Real-world scenario validation
- Complete migration workflow

**Example Structure:**
- Users collection with profiles, addresses
- Products collection with categories, variants
- Orders collection with line items (subcollection)
- Reviews collection with ratings and comments
- Complex relationships and nested data

**Planned Files:**
- Generator: `src/level9_complex.ts`
- Test: `../functional-tests/level9_tests.rs`

---

## Implementation Status

| Level | Name | Status | Generator | Tests |
|-------|------|--------|-----------|-------|
| 0 | Tiny Primitives | ✅ Complete | `tiny.ts` | `tiny_tests.rs` |
| 1 | Two Primitives | 📋 Planned | - | - |
| 2 | Two Rows | 📋 Planned | - | - |
| 3 | Two Primitives + Two Rows | 📋 Planned | - | - |
| 4 | Arrays | 📋 Planned | - | - |
| 5 | Nested Objects | 📋 Planned | - | - |
| 6 | Arrays of Objects | 📋 Planned | - | - |
| 7 | Multiple Collections | 📋 Planned | - | - |
| 8 | Subcollections | 📋 Planned | - | - |
| 9 | Complex Real-World | 📋 Planned | - | - |

## File Organization

```
tests/data-generator/
├── SAMPLE_DATA_PLAN.md          # This document
├── src/
│   ├── tiny.ts                   # ✅ Level 0
│   ├── level1_two_primitives.ts  # 📋 Level 1
│   ├── level2_two_rows.ts        # 📋 Level 2
│   ├── level3_two_primitives_two_rows.ts  # 📋 Level 3
│   ├── level4_arrays.ts          # 📋 Level 4
│   ├── level5_nested_objects.ts  # 📋 Level 5
│   ├── level6_arrays_of_objects.ts  # 📋 Level 6
│   ├── level7_relationships.ts   # 📋 Level 7
│   ├── level8_subcollections.ts  # 📋 Level 8
│   ├── level9_complex.ts        #  📋 Level 9
│   └── main.ts                   # Current complex example (to be refactored)
└── ../functional-tests/
    ├── tiny_tests.rs             # ✅ Level 0 tests
    ├── level1_tests.rs           # 📋 Level 1 tests
    ├── level2_tests.rs           # 📋 Level 2 tests
    ├── level3_tests.rs           # 📋 Level 3 tests
    ├── level4_tests.rs           # 📋 Level 4 tests
    ├── level5_tests.rs           # 📋 Level 5 tests
    ├── level6_tests.rs           # 📋 Level 6 tests
    ├── level7_tests.rs           # 📋 Level 7 tests
    ├── level8_tests.rs           # 📋 Level 8 tests
    └── level9_tests.rs           # 📋 Level 9 tests
```

## Usage

### Generating Sample Data

Each level can be generated independently:

```bash
# Generate Level 0 (Tiny Primitives)
npm run generate:level0

# Generate Level 1 (Two Primitives)
npm run generate:level1

# Generate all levels up to a specific level
npm run generate:up-to-level3

# Generate all levels
npm run generate:all
```

### Running Tests

Tests are organized by level:

```bash
# Run tests for a specific level
cargo test --test functional_tests level0
cargo test --test functional_tests level1

# Run all sample data tests
cargo test --test functional_tests sample_data
```

## Next Steps

1. **Implement Level 1**: Two Primitives (Two Fields, Single Row)
   - Create `src/level1_two_primitives.ts`
   - Create `../functional-tests/level1_tests.rs`
   - Update `package.json` with generation script

2. **Implement Level 2**: Two Rows (Single Field, Multiple Documents)
   - Create `src/level2_two_rows.ts`
   - Create `../functional-tests/level2_tests.rs`

3. **Continue progression**: Implement each level sequentially, building upon previous levels

4. **Documentation**: Update this document as each level is completed

## Testing Strategy

Each level should include tests for:
- **Parser Tests**: Verify correct parsing of the data structure
- **Schema Analysis Tests**: Validate schema detection and type inference
- **Normalization Tests**: Check proper table/column generation
- **Data Integrity Tests**: Ensure data is correctly transformed
- **Edge Case Tests**: Handle nulls, empty arrays, missing fields

## Notes

- Each level builds upon previous levels, ensuring comprehensive coverage
- Levels can be used independently for targeted testing
- The progression allows for incremental development and validation
- Real-world complexity (Level 9) combines all patterns for integration testing
