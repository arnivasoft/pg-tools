-- Bakım hedefi: indeks (`i`; PG 11+ bölümlenmiş `I`). $1 = pg_class oid'i (yoksa satır yok → NotFound).
-- `table_name`: indeksin ait olduğu tablo (aynı şema). Adlar ham metin.
SELECT n.nspname::text AS schema,
       c.relname::text AS name,
       c.relkind::text AS relkind,
       t.relname::text AS table_name,
       (n.nspname IN ('pg_catalog', 'information_schema') OR n.nspname LIKE 'pg\_toast%') AS is_system,
       (n.nspname LIKE 'pg\_temp\_%' OR n.nspname LIKE 'pg\_toast\_temp\_%') AS is_temp,
       ARRAY[]::text[] AS indexes,
       NULL::text AS cluster_index
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_index x ON x.indexrelid = c.oid
LEFT JOIN pg_class t ON t.oid = x.indrelid
WHERE c.oid = $1
