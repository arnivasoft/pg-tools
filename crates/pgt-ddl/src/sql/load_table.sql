-- Tablo başlığı. $1 = tablo (pg_class) oid'i; yalnızca sıradan ('r') ve bölümlenmiş ('p') tablo.
-- tablespace: 0 ya da pg_default → NULL. fillfactor ve diğer depolama seçenekleri reloptions'tan (diğerleri
-- "desteklenmeyen" listesine). inherits: bölüm OLMAYAN kalıtım çocuğu (bölümün ebeveyni pg_inherits'te de görünür).
SELECT n.nspname::text AS schema,
       c.relname::text AS name,
       pg_get_userbyid(c.relowner)::text AS owner,
       (SELECT s.spcname::text FROM pg_tablespace s WHERE s.oid = c.reltablespace AND s.spcname <> 'pg_default') AS tablespace,
       c.relpersistence::text AS persistence,
       (SELECT substring(o FROM 12) FROM unnest(c.reloptions) AS o WHERE o LIKE 'fillfactor=%' LIMIT 1) AS fillfactor,
       ARRAY(SELECT o FROM unnest(c.reloptions) AS o WHERE o NOT LIKE 'fillfactor=%' ORDER BY o)::text[] AS other_options,
       CASE WHEN c.reloftype = 0 THEN NULL ELSE format_type(c.reloftype, NULL)::text END AS of_type,
       obj_description(c.oid, 'pg_class') AS comment,
       c.relispartition AS is_partition,
       (NOT c.relispartition AND EXISTS (SELECT 1 FROM pg_inherits i WHERE i.inhrelid = c.oid)) AS inherits,
       CASE WHEN c.relkind = 'p' THEN pg_get_partkeydef(c.oid) END AS partition_text,
       (left(n.nspname, 3) = 'pg_' OR n.nspname = 'information_schema') AS is_system
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.oid = $1 AND c.relkind IN ('r', 'p')
