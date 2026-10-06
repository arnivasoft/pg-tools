SELECT pid::int4, NULL::int4 AS leader_pid,
       datname::text AS database, usename::text AS "user", application_name::text AS application,
       client_addr::text AS client_addr, backend_type::text AS backend_type, state::text AS state,
       wait_event_type::text, wait_event::text,
       extract(epoch FROM now() - backend_start)::float8 AS backend_age_secs,
       extract(epoch FROM now() - xact_start)::float8    AS xact_age_secs,
       extract(epoch FROM now() - query_start)::float8   AS query_age_secs,
       extract(epoch FROM now() - state_change)::float8  AS state_age_secs,
       left(query, 4096)::text AS query,
       (pid = pg_backend_pid()) AS is_self
FROM pg_catalog.pg_stat_activity
WHERE backend_type IS DISTINCT FROM 'walsender' AND ($1::text IS NULL OR datname = $1::text)
ORDER BY (state = 'active') DESC NULLS LAST, query_start NULLS LAST, pid
LIMIT 5001
