SELECT t.oid, t.tgname::text AS name,
       CASE t.tgenabled WHEN 'D' THEN 'disabled' WHEN 'R' THEN 'replica' WHEN 'A' THEN 'always' END AS detail,
       true AS leaf
FROM pg_trigger t
WHERE t.tgrelid = $1 AND NOT t.tgisinternal
ORDER BY t.tgname COLLATE "C"
