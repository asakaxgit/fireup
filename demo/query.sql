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
\echo '  The documented fidelity gap, in the stored row: the export parser'
\echo '  tags a timestamp instead of producing a timestampValue, so the'
\echo '  document plane stores a map. See docs/local/END_TO_END.md.'
SELECT document_id,
       (SELECT string_agg(value_type, ', ')
          FROM jsonb_object_keys(fields -> 'created_at') AS value_type)
         AS "created_at is stored as",
       (SELECT string_agg(field, ', ' ORDER BY field)
          FROM jsonb_object_keys(fields -> 'created_at' -> 'mapValue' -> 'fields') AS field)
         AS "whose fields are"
  FROM fireup_local_documents
 WHERE document_id = 'ada';
