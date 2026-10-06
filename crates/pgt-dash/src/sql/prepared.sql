SELECT gid::text, transaction::text AS transaction, owner::text, database::text,
       extract(epoch FROM now() - prepared)::float8 AS age_secs
FROM pg_catalog.pg_prepared_xacts
ORDER BY prepared
