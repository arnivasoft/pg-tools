SELECT l.pid::int4, l.locktype::text, l.mode::text, l.granted,
       CASE WHEN l.relation IS NOT NULL
             AND l.database = (SELECT oid FROM pg_catalog.pg_database WHERE datname = current_database())
            THEN l.relation::regclass::text END AS object,
       l.database::int8 AS database_oid
FROM pg_catalog.pg_locks l
JOIN pg_catalog.pg_stat_activity a ON a.pid = l.pid
WHERE l.pid <> pg_backend_pid() AND ($1::text IS NULL OR a.datname = $1::text)
ORDER BY l.granted, l.pid, l.locktype, l.mode
LIMIT 1001
