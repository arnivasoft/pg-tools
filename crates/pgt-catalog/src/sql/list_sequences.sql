SELECT c.oid, c.relname::text AS name, format_type(s.seqtypid, NULL) AS detail, true AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'e') AS from_extension
FROM pg_class c
JOIN pg_sequence s ON s.seqrelid = c.oid
WHERE c.relnamespace = $1 AND c.relkind = 'S'
  AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'i')
ORDER BY c.relname::text COLLATE "C"
