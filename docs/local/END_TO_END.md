# End to end: an export becomes SQL *and* a live Firestore API

One managed Firestore/Datastore export. One PostgreSQL database. Two ways to
read it — `SELECT`, and an official Firebase/Google Cloud SDK.

This is the workflow that joins Fireup's two halves: the **migrator**
(`fireup import`, export → normalized SQL) and the **local data plane**
(`fireup local`, the published `google.firestore.v1` / `google.datastore.v1`
gRPC APIs plus a subset of Identity Toolkit). They are not two products that
happen to live in one repository: they share the export parser, and they can
share a database.

Run it:

```bash
docker compose up -d postgres
scripts/demo-export-to-sql.sh
```

Everything below is what that script does, and what each step is evidence of.

---

## The shape of it

```
                  ┌──────────────────────────────┐
                  │  managed export (LevelDB log)│
                  └───────────────┬──────────────┘
                                  │   one parser, src/leveldb_parser
                  ┌───────────────┴──────────────┐
                  ▼                              ▼
      fireup import                    fireup local seed
      normalized tables                fireup_local_documents
      "Users", "Orders"                (jsonb, Firestore value shape)
                  │                              │
                  │                              ▼
                  │                      fireup local  ──►  FIRESTORE_EMULATOR_HOST
                  │                      (one port)         DATASTORE_EMULATOR_HOST
                  │                              │          FIREBASE_AUTH_EMULATOR_HOST
                  ▼                              ▼
             psql / BI / ORM          @google-cloud/firestore, firebase-admin, …
                  └───────────── one PostgreSQL database ────┘
```

## Why both planes, and when to use which

| | `fireup import` → SQL tables | `fireup local seed` → document plane |
|---|---|---|
| Shape | Normalized relational tables, one per kind | One row per document, fields as `jsonb` |
| Read with | `psql`, an ORM, a BI tool, anything that speaks SQL | An official Firestore/Datastore SDK, unchanged |
| Types | Mapped to real PostgreSQL types: `timestamptz`, `bytea`, `point`, `numeric` | The public Firestore value shape, including the four Datastore types [below](#what-seeding-does-with-datastore-value-types) |
| Good for | Reporting, the actual migration, schema review, constraints | Running existing application code locally, tests, a staged migration |

You usually want both, and at the same time: that is the point of pointing them
at one database. A team mid-migration can run its unmodified Firestore client
code against `fireup local` while its new SQL queries run against the imported
tables — same data, same connection string, no second copy to keep in sync.

---

## Step 1 — Does the export read?

```bash
fireup validate --backup-file tests/fixtures/export-onestore
```

Every later step reuses this one parser, so a failure here is the only failure
worth debugging first. On the repo's fixture it reports 12 entities over 2
kinds, 0 errors. A real export is a directory tree, not a file: `validate`
resolves every `all_namespaces/kind_*/output-N` shard of every kind, so point
`--backup-file` at the export's root.

## Step 2 — Migrator: export → normalized SQL

```bash
fireup import \
  --backup-file ./export \
  --postgres-url "$POSTGRES_URL" \
  --drop-existing \
  --generate-ddl ./generated-schema.sql
```

This analyses the documents, normalizes them (1NF/2NF/3NF), emits DDL, creates
the tables, and imports the rows. `--generate-ddl` writes the schema out first
so you can read what Fireup chose before trusting it — worth doing once per
export shape.

On the fixture that produces `"Users"` and `"Orders"`, keyed by `key_path`, with
`parent_key_path` carrying the export's key hierarchy. Types come out as real
PostgreSQL types, so SQL works on them:

```sql
SELECT u.key_name, u.score, count(o.key_path) AS orders, sum(o.total_cents)
  FROM "Users" u
  LEFT JOIN "Orders" o ON o.parent_key_path = u.key_path
 WHERE u.active
 GROUP BY u.key_name, u.score;
```

Two things to know about the output:

- **Join on `parent_key_path`.** The normalizer also emits a junction table for
  a key-reference property (`"Orders_Users_lookup"` for `Orders.user`), but
  `import` does not populate it — it comes out empty. `parent_key_path` and the
  reference column (`"Orders".user`) both carry the relationship.
- **Identifiers are quoted and case-sensitive**, because kinds are:
  `"Users"`, not `users`.

## Step 3 — Local data plane: the same export → documents

```bash
fireup local seed \
  --backup-file ./export \
  --postgres-url "$POSTGRES_URL" \
  --project-id fireup-demo
```

Same parser, different destination: the `fireup_local_documents` table that
`fireup local` serves from. Writes are upserts, so seeding twice is the same as
seeding once, and a seed that could not write every parsed document **exits
non-zero** — a partially seeded store is indistinguishable from a complete one
from the outside.

Key paths become document paths, so the export's hierarchy survives as
Firestore structure. An `Orders` entity whose key is `Users/ada > Orders/1001`
seeds as:

```
projects/fireup-demo/databases/(default)/documents/Users/ada/Orders/1001
```

which an SDK reads as the subcollection `Users/ada/Orders`. Nothing flattens.

Value types survive too, including the four Datastore types JSON cannot express
— see [below](#what-seeding-does-with-datastore-value-types).

Use the same `--project-id` here, in step 4, and in your SDK. They have to
agree: the server answers for one project, and a resource name contains it.

## Step 4 — Serve it

```bash
fireup local --port 4591 --postgres-url "$POSTGRES_URL" --project-id fireup-demo
curl -fsS http://127.0.0.1:4591/health
```

`/health` asks the store, so a `200` means the server can reach the database
too — which is what makes it usable as a readiness gate in a script or a
container health check. See [`README.md`](README.md) for every flag, the
one-port/two-protocol trick, and Docker Compose.

## Step 5 — Official SDKs read it

```bash
export FIRESTORE_EMULATOR_HOST=127.0.0.1:4591
export DATASTORE_EMULATOR_HOST=127.0.0.1:4591
```

That is the entire integration. Any official SDK honours those variables, so
application code needs no Fireup-specific branch, no shim, and no wrapper:

```js
import { Firestore } from '@google-cloud/firestore'

const db = new Firestore({ projectId: 'fireup-demo' })

const ada = await db.doc('Users/ada').get()
const orders = await db.collection('Users/ada/Orders').get()
const active = await db.collection('Users').where('active', '==', true).orderBy('score', 'desc').get()
```

The query is compiled and run by `fireup local`, not filtered client-side. What
query shapes are served, and which are an explicit `UNIMPLEMENTED`, is the
[fidelity matrix](README.md#fidelity-matrix).

**The same rows through the Datastore API.** Kinds are collections and key paths
are document paths, so this is not a second copy — it is the same row:

```js
import { Datastore, PropertyFilter } from '@google-cloud/datastore'

const ds = new Datastore({ projectId: 'fireup-demo' })
const [ada] = await ds.get(ds.key(['Users', 'ada']))
const [rows] = await ds.runQuery(ds.createQuery('Users').filter(new PropertyFilter('active', '=', true)))
```

A document written through Firestore is immediately readable through Datastore,
and the other way round. `demo/read-with-sdk.mjs` asserts exactly that.

## Step 6 — SQL on the same database

The document plane is an ordinary PostgreSQL table, so the SDK's writes are
visible to SQL with no export step and no sync job:

```sql
SELECT document_id,
       fields -> 'currency'    ->> 'stringValue'  AS currency,
       (fields -> 'total_cents' ->> 'integerValue')::bigint AS total_cents
  FROM fireup_local_documents
 WHERE collection_id = 'Orders';
```

`fields` is `jsonb` in the **public Firestore REST value shape** —
`{"total_cents": {"integerValue": "12999"}}` — which is why the extraction needs
the value-type key. That shape is deliberate: it is the documented wire
representation, it round-trips every type an SDK can store (including `int64`
past 2^53, which is why integers are strings), and it keeps one encoding between
the store and the API.

`demo/query.sql` is the full set, including the one row the SDK wrote during
step 5 and the normalized tables from step 2, side by side in one database.

---

## What seeding does with Datastore value types

Four Datastore types have no native JSON form, so the export parser decodes them
into a *tagged map* — `{"__fireup_type__": "timestamp", "rfc3339": "…"}` —
rather than erasing them. Both planes read those tags: `fireup import` maps them
onto `bytea`, `timestamptz`, `point`, and `text`, and **`fireup local seed`
restores the matching Firestore value type**, so no client needs to know that
`__fireup_type__` exists.

| Export property | Seeded value type | What an SDK reads |
|---|---|---|
| timestamp | `timestampValue` | `Timestamp` |
| blob | `bytesValue` | `Buffer` / `Bytes` |
| geo point | `geoPointValue` | `GeoPoint` |
| key reference | `referenceValue` | `DocumentReference` |

Strings, numbers, booleans, nulls, arrays, and ordinary nested maps seed from
their JSON shape, as before. `demo/read-with-sdk.mjs` asserts all four through
the official client library, and `demo/query.sql` shows the same four in the
stored `jsonb`.

Two details that are deliberate rather than incidental:

- **Only the tag decides the type.** A document holding the literal text of a
  date is not the same as one holding a timestamp, so nothing is inferred from a
  string's shape. A map that merely happens to carry a `__fireup_type__` key
  whose value Fireup does not know stays an ordinary `mapValue`.
- **A key reference becomes a resource name.** The tag carries a Datastore key
  path (`Users/ada`); a `referenceValue` is a full
  `projects/…/databases/…/documents/…` name. Seeding resolves it through the
  same key-to-document-path mapping it uses for a document's own name, so a
  reference points at exactly the document that key seeded as.

Three seeding limits, all deliberate refusals rather than silent results:

- **A tagged value whose payload cannot be restored fails its document.**
  Unparseable base64, a key path that is not a sequence of kind/id pairs, or a
  reference into a non-default namespace is reported, not seeded as a map.
- **Non-default namespaces are skipped, not merged.** Two namespaces can hold
  the same key path, and mapping both onto one document name would overwrite
  data.
- **A document that cannot be mapped fails the command.** `seed` prints each
  reason and exits non-zero.

Everything else that is not production Firestore — no realtime listeners, no
aggregations, no field transforms, no security rules, unsigned auth tokens — is
enumerated in the
[fidelity matrix](README.md#fidelity-matrix). The rule there holds here too:
**an unsupported feature is an explicit error, never a wrong answer.**

## Re-running, and cleaning up

The whole workflow is idempotent: `import --drop-existing` recreates its tables,
`seed` upserts, and the demo deletes the one document it creates before it reads.

```sql
-- The document plane only.
DELETE FROM fireup_local_documents WHERE project_id = 'fireup-demo';
-- Accounts, if you used the auth surface.
DELETE FROM fireup_local_auth_users WHERE project_id = 'fireup-demo';
```

`DROP TABLE "Users", "Orders", "Orders_Users_lookup";` removes the migrator's
output. The two planes never share a table, so dropping one leaves the other
intact.

## Where to go next

- [`docs/local/README.md`](README.md) — flags, the fidelity matrix, the auth
  surface, storage layout, tests
- [`scripts/demo-export-to-sql.sh`](../../scripts/demo-export-to-sql.sh) — this
  document, executable
- [`demo/read-with-sdk.mjs`](../../demo/read-with-sdk.mjs) and
  [`demo/query.sql`](../../demo/query.sql) — the two halves of step 5 and 6
- [`PROJECT_STATUS.md`](../../PROJECT_STATUS.md) — what is proven and what is
  not, including the live-GCP-export gap
- [`docs/licenses/FIREBASE_GCP_LOCAL.md`](../licenses/FIREBASE_GCP_LOCAL.md) —
  what is vendored, under what licence, and the compliance rule
