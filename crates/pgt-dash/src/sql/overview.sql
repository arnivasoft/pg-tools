SELECT pg_catalog.version() AS version,
       extract(epoch FROM now() - pg_postmaster_start_time())::float8 AS uptime_secs,
       current_setting('max_connections')::int8 AS max_connections,
       (SELECT count(*) FROM pg_catalog.pg_stat_activity
         WHERE backend_type = 'client backend' AND ($1::text IS NULL OR datname = $1::text))::int8 AS connections,
       (SELECT extract(epoch FROM now() - min(xact_start))::float8 FROM pg_catalog.pg_stat_activity
         WHERE xact_start IS NOT NULL AND pid <> pg_backend_pid()
           AND ($1::text IS NULL OR datname = $1::text)) AS oldest_xact_secs
