-- Şema var mı, sistem şeması mı? $1 = şema adı. Satır yoksa şema yok.
SELECT (n.nspname = 'information_schema'::pg_catalog.name
        OR pg_catalog.left(n.nspname::pg_catalog.text, 3) = 'pg_') AS is_system
FROM pg_catalog.pg_namespace n
WHERE n.nspname = $1::pg_catalog.text
