SELECT t.oid, t.typname::text AS name, format_type(t.typbasetype, t.typtypmod) AS detail, true AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_type'::regclass AND d.objid = t.oid AND d.deptype = 'e') AS from_extension
FROM pg_type t
WHERE t.typnamespace = $1 AND t.typtype = 'd'
ORDER BY t.typname COLLATE "C"
