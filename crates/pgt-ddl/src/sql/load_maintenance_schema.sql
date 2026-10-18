-- Bakım hedefi: şema. $1 = pg_namespace oid'i (yoksa satır yok → NotFound). Ad ham metin.
SELECT NULL::text AS schema,
       n.nspname::text AS name,
       'n'::text AS relkind,
       NULL::text AS table_name,
       (n.nspname IN ('pg_catalog', 'information_schema') OR n.nspname LIKE 'pg\_toast%') AS is_system,
       (n.nspname LIKE 'pg\_temp\_%' OR n.nspname LIKE 'pg\_toast\_temp\_%') AS is_temp,
       ARRAY[]::text[] AS indexes,
       NULL::text AS cluster_index
FROM pg_namespace n
WHERE n.oid = $1
