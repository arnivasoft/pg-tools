WITH b AS (
  SELECT a.pid, pg_blocking_pids(a.pid) AS blocked_by
  FROM pg_catalog.pg_stat_activity a
  WHERE a.pid <> pg_backend_pid() AND ($1::text IS NULL OR a.datname = $1::text)
), involved AS (
  SELECT pid FROM b WHERE cardinality(blocked_by) > 0
  UNION
  SELECT DISTINCT unnest(blocked_by) FROM b WHERE cardinality(blocked_by) > 0
)
SELECT a.pid::int4, a.usename::text AS "user", a.application_name::text AS application, a.state::text AS state,
       left(a.query, 1024)::text AS query,
       pg_blocking_pids(a.pid)::int4[] AS blocked_by,
       extract(epoch FROM now() - a.state_change)::float8 AS wait_secs
FROM pg_catalog.pg_stat_activity a
JOIN involved i ON i.pid = a.pid
ORDER BY a.pid
