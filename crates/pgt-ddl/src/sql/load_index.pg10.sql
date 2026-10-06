-- İndeks başlığı. $1 = indeks (pg_class) oid'i. PG 10: indnkeyatts yok; INCLUDE yok, tüm kolonlar anahtardır.
-- WHERE: pg_get_expr(.., true) gereksiz parantezleri bırakır. constraint_name: yalnızca PK/UNIQUE/EXCLUDE'a ait indeks
-- (FK'ler de conindid taşır; contype ve conrelid ile elenir).
SELECT n.nspname::text AS schema,
       t.relname::text AS table_name,
       c.relname::text AS name,
       am.amname::text AS method,
       i.indisunique AS is_unique,
       i.indnatts::int4 AS nkeyatts,
       pg_get_expr(i.indpred, i.indrelid, true) AS where_clause,
       CASE WHEN c.reltablespace = 0 THEN NULL ELSE (SELECT s.spcname::text FROM pg_tablespace s WHERE s.oid = c.reltablespace) END AS tablespace,
       (SELECT substring(o FROM 12) FROM unnest(c.reloptions) AS o WHERE o LIKE 'fillfactor=%' LIMIT 1) AS fillfactor,
       obj_description(c.oid, 'pg_class') AS comment,
       (SELECT con.conname::text FROM pg_constraint con
         WHERE con.conindid = c.oid AND con.conrelid = i.indrelid AND con.contype IN ('p', 'u', 'x')
         LIMIT 1) AS constraint_name,
       (left(n.nspname, 3) = 'pg_' OR n.nspname = 'information_schema') AS is_system
FROM pg_index i
JOIN pg_class c ON c.oid = i.indexrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_class t ON t.oid = i.indrelid
JOIN pg_am am ON am.oid = c.relam
WHERE i.indexrelid = $1 AND c.relkind IN ('i', 'I')
