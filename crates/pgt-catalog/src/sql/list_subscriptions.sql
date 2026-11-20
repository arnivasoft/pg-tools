SELECT s.oid, s.subname::text AS name, CASE WHEN s.subenabled THEN 'etkin' ELSE 'devre dışı' END AS detail, true AS leaf
FROM pg_subscription s
WHERE s.subdbid = (SELECT d.oid FROM pg_database d WHERE d.datname = current_database())
ORDER BY s.subname::text COLLATE "C"
