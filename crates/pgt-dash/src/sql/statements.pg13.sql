SELECT queryid::text AS query_id, left(query, 2048)::text AS query, calls::int8,
       total_exec_time::float8 AS total_ms, mean_exec_time::float8 AS mean_ms, rows::int8,
       shared_blks_hit::int8, shared_blks_read::int8
FROM pg_stat_statements
WHERE $1::text IS NULL OR dbid = (SELECT oid FROM pg_catalog.pg_database WHERE datname = $1::text)
