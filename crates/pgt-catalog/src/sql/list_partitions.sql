SELECT c.oid, c.relname::text AS name, pg_get_expr(c.relpartbound, c.oid) AS detail,
       false AS leaf, c.relkind = 'p' AS partitioned
FROM pg_inherits i
JOIN pg_class c ON c.oid = i.inhrelid
WHERE i.inhparent = $1 AND c.relispartition
ORDER BY c.relname::text COLLATE "C"
