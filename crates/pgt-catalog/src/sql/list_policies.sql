SELECT p.oid, p.polname::text AS name,
       CASE p.polcmd WHEN 'r' THEN 'select' WHEN 'a' THEN 'insert' WHEN 'w' THEN 'update' WHEN 'd' THEN 'delete' ELSE 'all' END AS detail,
       true AS leaf
FROM pg_policy p
WHERE p.polrelid = $1
ORDER BY p.polname COLLATE "C"
