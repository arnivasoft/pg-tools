SELECT n.oid, n.nspname::text AS name,
       (n.nspname IN ('pg_catalog', 'information_schema')
        OR n.nspname LIKE 'pg\_toast%'
        OR n.nspname LIKE 'pg\_temp\_%') AS is_system
FROM pg_namespace n
ORDER BY is_system, n.nspname::text COLLATE "C"
