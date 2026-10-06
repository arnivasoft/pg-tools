-- Eklenti tanımı. $1 = pg_extension oid'i.
SELECT e.extname::text AS name,
       n.nspname::text AS schema,
       e.extversion::text AS version,
       (left(n.nspname, 3) = 'pg_' OR n.nspname = 'information_schema') AS is_system
FROM pg_extension e
JOIN pg_namespace n ON n.oid = e.extnamespace
WHERE e.oid = $1
