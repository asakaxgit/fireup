-- The SQL half of the end-to-end demo.
--
-- Every query below runs against the *same* PostgreSQL database the SDK half
-- just talked to. `fireup import` put the export into normalized relational
-- tables; `fireup local seed` put the same export into the document plane's
-- table. Both are ordinary tables in one database, so one connection — and one
-- transaction, if you want it — sees both.
--
-- Run it through scripts/demo-export-to-sql.sh.

\pset border 2
\timing off

\echo
\echo '  Both planes, one database:'
SELECT c.relname AS "table",
       CASE
         WHEN c.relname LIKE 'fireup_local_%' THEN 'local data plane (fireup local)'
         ELSE 'migrator output (fireup import)'
       END AS "written by",
       (SELECT count(*) FROM pg_catalog.pg_attribute a
         WHERE a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped) AS columns
  FROM pg_catalog.pg_class c
  JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
 WHERE c.relkind = 'r' AND n.nspname = current_schema()
 ORDER BY 2 DESC, 1;

\echo
\echo '  Migrator output — normalized SQL, queried as SQL:'
SELECT u.key_name AS "user",
       u.score,
       count(o.key_path) AS orders,
       sum(o.total_cents) AS total_cents,
       u.address ->> 'city' AS city
  FROM "Users" u
  LEFT JOIN "Orders" o ON o.parent_key_path = u.key_path
 WHERE u.active
 GROUP BY u.key_name, u.score, u.address
 ORDER BY u.score DESC;

\echo
\echo '  Local data plane — the same export as documents, plus the one'
\echo '  document the SDK wrote a moment ago:'
SELECT collection_id AS collection, count(*) AS documents
  FROM fireup_local_documents
 GROUP BY collection_id
 ORDER BY collection_id;

\echo
\echo '  Document fields are jsonb in the public Firestore value shape,'
\echo '  so SQL can read what an SDK wrote:'
SELECT document_id,
       fields -> 'currency' ->> 'stringValue' AS currency,
       (fields -> 'total_cents' ->> 'integerValue')::bigint AS total_cents,
       fields -> 'note' ->> 'stringValue' AS note
  FROM fireup_local_documents
 WHERE collection_id = 'Orders'
 ORDER BY (fields -> 'note' ->> 'stringValue') IS NULL, document_id;

\echo
\echo '  The row the official Firestore SDK wrote a moment ago:'
SELECT name, update_time_seconds
  FROM fireup_local_documents
 WHERE document_id = 'demo-sdk-write';

\echo
\echo '  The four Datastore types JSON has no native form for. The export'
\echo '  parser hands them to seed as __fireup_type__-tagged maps; seeding'
\echo '  reads those tags, so the row holds the real value types — the same'
\echo '  ones the SDK read a moment ago. An ordinary object stays a mapValue.'
SELECT f.field,
       v.key AS "stored value type",
       left(v.value #>> '{}', 54) AS "value"
  FROM fireup_local_documents d
 CROSS JOIN (VALUES ('created_at'), ('avatar'), ('location'), ('manager'), ('address'))
         AS f(field)
 -- Each field is a single-key object: the value-type tag and its payload.
 CROSS JOIN LATERAL jsonb_each(d.fields -> f.field) AS v
 WHERE d.collection_id = 'Users' AND d.document_id = 'grace'
 ORDER BY f.field;
