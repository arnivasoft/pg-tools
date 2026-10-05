SELECT t.oid, t.typname::text AS name,
       CASE t.typtype WHEN 'e' THEN 'enum' WHEN 'c' THEN 'composite' WHEN 'r' THEN 'range' ELSE 'base' END AS detail,
       true AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_type'::regclass AND d.objid = t.oid AND d.deptype = 'e') AS from_extension
FROM pg_type t
WHERE t.typnamespace = $1
  AND t.typtype IN ('e', 'c', 'r', 'b')
  AND (t.typtype <> 'c' OR EXISTS (SELECT 1 FROM pg_class c WHERE c.oid = t.typrelid AND c.relkind = 'c'))
  AND NOT EXISTS (SELECT 1 FROM pg_type el WHERE el.oid = t.typelem AND el.typarray = t.oid)
ORDER BY t.typname COLLATE "C"
