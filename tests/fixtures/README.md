# Firestore / Datastore managed-export fixtures

These fixtures reproduce the on-disk shape and binary format of a real
`gcloud firestore export` / `gcloud datastore export`, so that `fireup validate`,
`analyze`, and `import` can be exercised against something representative
without needing a live GCP project.

## Trees

| Tree | Record payload | Purpose |
| --- | --- | --- |
| `export-onestore/` | legacy `storage_onestore_v3.EntityProto` | What a real managed export actually contains |
| `export-v1-entity/` | `google.datastore.v1.Entity` | The shape `src/leveldb_parser/protobuf_entity.rs` currently decodes |

Both trees carry **identical** LevelDB framing and the same logical dataset, so a
failure that reproduces in both is a framing bug, while a failure that reproduces
only in `export-onestore/` is a payload/proto bug. That separation is the point
of having two trees — keep it when extending them.

## Layout

```
export-onestore/
├── 2026-03-14T09:26:53_12345.overall_export_metadata
└── all_namespaces/
    ├── kind_Orders/
    │   ├── all_namespaces_kind_Orders.export_metadata
    │   └── output-0
    └── kind_Users/
        ├── all_namespaces_kind_Users.export_metadata
        └── output-0
```

Note that entities are split **per kind, across separate `output-N` files**. A
reader that resolves an export to one file sees one kind at most.

## Dataset

12 entities: 5 `Users` and 7 `Orders` (`Orders` are children of `Users`, so their
keys have two path elements). Deliberate edge cases:

- string key names (`ada`), a numeric key id (`4503599627370496`, beyond `int32`),
  and a non-ASCII key name (`unicode-ユーザー`)
- a sparse field (`nickname`, null on 4 of 5 users)
- negative `int64` and negative `double` values
- empty string, empty blob, and empty array values
- a nested map (`address`), an array (`tags`), a key reference (`manager`),
  a geo point (`location`), and a timestamp (`created_at`)
- a ~45 KB blob on one user, so that record **must** fragment across 32 KiB
  LevelDB blocks (FIRST/LAST) — this is the only coverage of the fragment
  reassembly path

## Binary format

Record framing is the LevelDB **log** format (`leveldb/db/log_format.h`), not an
SSTable:

- 32768-byte blocks; a trailing remainder shorter than a 7-byte header is zero padded
- 7-byte record header: 4-byte checksum (LE), 2-byte payload length (LE), 1-byte type
- record types: `1=FULL`, `2=FIRST`, `3=MIDDLE`, `4=LAST`
- checksum is **masked CRC32C** — Castagnoli polynomial `0x82F63B78` over
  `type_byte || payload`, then masked with
  `((crc >> 15) | (crc << 17)) + 0xA282EAD8`

Payloads in `export-onestore/` are proto2 `EntityProto` messages whose field
numbering is unrelated to `google.datastore.v1`:

```
EntityProto   key = 13 (Reference), property = 14 (repeated Property),
              raw_property = 15, entity_group = 16 (Path)
Reference     app = 13, path = 14 (Path), name_space = 20, database_id = 21
Path          repeated group Element = 1 { type = 2, id = 3, name = 4 }
Property      meaning = 1, meaning_uri = 2, name = 3, multiple = 4, value = 5
PropertyValue int64Value = 1, booleanValue = 2, stringValue = 3, doubleValue = 4,
              group PointValue = 5 { x = 6, y = 7 },
              group UserValue = 8 { ... },
              group ReferenceValue = 12 { app = 13,
                  repeated group PathElement = 14 { type = 15, id = 16, name = 17 } }
```

Two properties of that schema matter for any decoder:

1. **Proto2 groups** (wire types 3/4) are used for `Path.Element`,
   `PropertyValue.PointValue`, and `PropertyValue.ReferenceValue`. `prost` does
   not support groups, so these cannot be expressed as a `prost` message.
2. **Types are carried by `Property.meaning`**, not by the value field. A
   timestamp is `int64Value` + `meaning = 7 (GD_WHEN)` in **microseconds**; a blob
   is `stringValue` + `meaning = 14 (BLOB)`; text is `stringValue` +
   `meaning = 15 (TEXT)`; a nested map is `stringValue` holding a nested
   serialized `EntityProto` + `meaning = 19 (ENTITY_PROTO)`; an empty list is
   `meaning = 24 (EMPTY_LIST)`. Arrays are *repeated* `Property` entries sharing
   one `name` with `multiple = true`. Ignoring `meaning` silently turns every
   timestamp into an integer and every blob into mojibake.

## Regenerating

```bash
python3 tests/fixtures/generate_datastore_export.py   # rewrites both trees in place
python3 tests/fixtures/verify_datastore_export.py     # independent self-check
```

`verify_datastore_export.py` re-reads the fixtures with a reader written from the
spec rather than from the generator's helpers, and validates every CRC. Run it
after any change so a fireup failure can never be blamed on the fixture.

The generator also accepts `--crc ieee`, which emits unmasked IEEE CRC-32
checksums — what `LevelDBReader` computed before
[SAK-141](/SAK/issues/SAK-141). fireup now rejects that variant, which is the
point: it must never be committed as a fixture, and it is only useful as a
scratch-directory diagnostic for isolating downstream behaviour from framing:

```bash
python3 tests/fixtures/generate_datastore_export.py /tmp/diag --crc ieee
```

## Tests

`tests/real_export_validation.rs` consumes these fixtures: guards that pin the
fixtures as spec-correct with a reader written independently of `src/`, plus
acceptance tests that read the fixtures through fireup itself. All of them run
by default — `cargo test --test real_export_validation`.

`tests/ddl_postgres_load_tests.rs` takes the next step and generates DDL from
these fixtures, then loads it with `psql --variable=ON_ERROR_STOP=1` and reads
the resulting catalog back, so PostgreSQL itself decides whether the generated
schema is valid. Asserting on the DDL string could not catch what this does: a
property named `user` emitted unquoted is a `syntax error at or near "user"`
that aborts the rest of the script. Each test loads into its own randomly named
schema and drops it even on failure, so runs do not collide. It needs a
reachable database and is skipped without one:

```bash
FIREUP_TEST_POSTGRES_URL='postgresql:///fireup_test?host=/var/run/postgresql' \
  cargo test --test ddl_postgres_load_tests
```

The deliberate edge cases above are what give that test its teeth — `user` as a
reserved-word column, `nickname` for nullability, the blob/geo-point/timestamp
properties for type fidelity, and the two-element `Orders` keys for the
ancestor link. Keep them when extending the dataset.

Which decoder handles a payload is decided per record, so both trees are read by
the same code path:

| Payload | Decoder |
| --- | --- |
| legacy `EntityProto` (field 13 `Reference` key) | `src/leveldb_parser/entity_proto.rs`, a hand-written wire decoder (proto2 groups rule out `prost`) |
| `datastore.v1.Entity` | `src/leveldb_parser/protobuf_entity.rs`, via `prost` |
