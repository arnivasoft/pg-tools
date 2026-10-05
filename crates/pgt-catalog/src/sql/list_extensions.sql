SELECT e.oid, e.extname::text AS name, e.extversion AS detail, true AS leaf
FROM pg_extension e
ORDER BY e.extname COLLATE "C"
