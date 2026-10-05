SELECT c.relname::text AS name, n.nspname::text AS schema, c.oid, pg_get_userbyid(c.relowner)::text AS owner,
       obj_description(c.oid, 'pg_class') AS comment,
       CASE c.relkind WHEN 'p' THEN 'Partitioned table' ELSE 'Table' END AS kind,
       CASE c.relpersistence WHEN 'u' THEN 'unlogged' WHEN 't' THEN 'temporary' ELSE 'permanent' END AS persistence,
       COALESCE(t.spcname::text, 'pg_default') AS tablespace,
       array_to_string(c.reloptions, ', ') AS options,
       c.reltuples::float8 AS reltuples,
       pg_size_pretty(pg_total_relation_size(c.oid)) AS total_size,
       pg_size_pretty(pg_relation_size(c.oid)) AS table_size,
       pg_size_pretty(pg_indexes_size(c.oid)) AS indexes_size,
       to_char(s.last_vacuum, 'YYYY-MM-DD HH24:MI:SS') AS last_vacuum,
       to_char(s.last_autovacuum, 'YYYY-MM-DD HH24:MI:SS') AS last_autovacuum,
       to_char(s.last_analyze, 'YYYY-MM-DD HH24:MI:SS') AS last_analyze,
       to_char(s.last_autoanalyze, 'YYYY-MM-DD HH24:MI:SS') AS last_autoanalyze
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_tablespace t ON t.oid = c.reltablespace
LEFT JOIN pg_stat_all_tables s ON s.relid = c.oid
WHERE c.oid = $1 AND c.relkind IN ('r', 'p')
