SELECT p.oid, p.proname::text || '(' || regexp_replace(pg_get_function_identity_arguments(p.oid), '(^|, )IN ', '\1', 'g') || ')' AS name,
       NULL::text AS detail, true AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype = 'e') AS from_extension
FROM pg_proc p
WHERE p.pronamespace = $1 AND p.prokind = 'p'
  AND NOT EXISTS (SELECT 1 FROM pg_depend i WHERE i.classid = 'pg_proc'::regclass AND i.objid = p.oid AND i.deptype = 'i')
ORDER BY p.proname COLLATE "C", p.oid
