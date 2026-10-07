-- Materialized view başlığı. $1 = MV (pg_class) oid'i; yalnızca relkind 'm' (başka tür → satır yok → NotFound).
-- query: pg_get_viewdef(.., true) (sondaki ';' Rust'ta soyulur). tablespace: 0 ya da pg_default → NULL.
-- fillfactor ve diğer depolama seçenekleri reloptions'tan (diğerleri "desteklenmeyen" listesine).
-- populated: relispopulated (WITH [NO] DATA). indexes: MV indeksleri (Index türüne ait; yeniden kurmada kaybolur), ad sırasıyla.
SELECT n.nspname::text AS schema,
       c.relname::text AS name,
       pg_get_userbyid(c.relowner)::text AS owner,
       pg_get_viewdef(c.oid, true) AS query,
       c.relpersistence::text AS persistence,
       (SELECT s.spcname::text FROM pg_tablespace s WHERE s.oid = c.reltablespace AND s.spcname <> 'pg_default') AS tablespace,
       (SELECT substring(o FROM 12) FROM unnest(c.reloptions) AS o WHERE o LIKE 'fillfactor=%' LIMIT 1) AS fillfactor,
       ARRAY(SELECT o FROM unnest(c.reloptions) AS o WHERE o NOT LIKE 'fillfactor=%' ORDER BY o COLLATE "C")::text[] AS other_options,
       c.relispopulated AS populated,
       obj_description(c.oid, 'pg_class') AS comment,
       ARRAY(SELECT ic.relname::text FROM pg_index i JOIN pg_class ic ON ic.oid = i.indexrelid
             WHERE i.indrelid = c.oid
             ORDER BY ic.relname::text COLLATE "C")::text[] AS indexes,
       (left(n.nspname, 3) = 'pg_' OR n.nspname = 'information_schema') AS is_system
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.oid = $1 AND c.relkind = 'm'
