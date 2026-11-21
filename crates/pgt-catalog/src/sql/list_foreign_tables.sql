SELECT c.oid, c.relname::text AS name, s.srvname::text AS detail, false AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'e') AS from_extension
FROM pg_class c
JOIN pg_foreign_table ft ON ft.ftrelid = c.oid
JOIN pg_foreign_server s ON s.oid = ft.ftserver
WHERE c.relnamespace = $1 AND c.relkind = 'f' AND NOT c.relispartition
ORDER BY c.relname::text COLLATE "C"
