SELECT c.oid, c.relname::text AS name, NULL::text AS detail, false AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'e') AS from_extension
FROM pg_class c
WHERE c.relnamespace = $1 AND c.relkind = 'v'
ORDER BY c.relname::text COLLATE "C"
