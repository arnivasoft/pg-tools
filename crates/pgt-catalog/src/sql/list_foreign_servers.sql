SELECT s.oid, s.srvname::text AS name, NULLIF(concat_ws(' · ', NULLIF(s.srvtype, ''), NULLIF(s.srvversion, '')), '') AS detail, false AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_foreign_server'::regclass AND d.objid = s.oid AND d.deptype = 'e') AS from_extension
FROM pg_foreign_server s
WHERE s.srvfdw = $1
ORDER BY s.srvname::text COLLATE "C"
