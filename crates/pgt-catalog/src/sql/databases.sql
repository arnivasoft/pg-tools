SELECT d.oid, d.datname::text AS name, d.datallowconn AS allow_conn
FROM pg_database d
WHERE NOT d.datistemplate
ORDER BY d.datname::text COLLATE "C"
