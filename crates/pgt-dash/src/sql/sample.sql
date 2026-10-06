WITH d AS (
  SELECT coalesce(sum(xact_commit), 0)::int8   AS xact_commit,
         coalesce(sum(xact_rollback), 0)::int8 AS xact_rollback,
         coalesce(sum(tup_returned), 0)::int8  AS tup_returned,
         coalesce(sum(tup_fetched), 0)::int8   AS tup_fetched,
         coalesce(sum(tup_inserted), 0)::int8  AS tup_inserted,
         coalesce(sum(tup_updated), 0)::int8   AS tup_updated,
         coalesce(sum(tup_deleted), 0)::int8   AS tup_deleted,
         coalesce(sum(blks_read), 0)::int8     AS blks_read,
         coalesce(sum(blks_hit), 0)::int8      AS blks_hit
  FROM pg_catalog.pg_stat_database
  WHERE $1::text IS NULL OR datname = $1::text
), a AS (
  SELECT count(*) FILTER (WHERE state = 'active')::int8 AS active,
         count(*) FILTER (WHERE state = 'idle')::int8 AS idle,
         count(*) FILTER (WHERE state LIKE 'idle in transaction%')::int8 AS idle_in_tx,
         count(*) FILTER (WHERE state IS DISTINCT FROM 'active' AND state IS DISTINCT FROM 'idle'
                            AND state NOT LIKE 'idle in transaction%')::int8 AS other
  FROM pg_catalog.pg_stat_activity
  WHERE backend_type = 'client backend' AND ($1::text IS NULL OR datname = $1::text)
)
SELECT extract(epoch FROM clock_timestamp())::float8 AS at, d.*, a.active, a.idle, a.idle_in_tx, a.other
FROM d, a
