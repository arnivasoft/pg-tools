SELECT c.oid, c.relname::text AS name, c.relkind = 'p' AS partitioned, c.reltuples::float8 AS reltuples
FROM pg_class c
WHERE c.relnamespace = $1
  AND c.relkind IN ('r', 'p')
  AND NOT c.relispartition
ORDER BY c.relname::text COLLATE "C"
