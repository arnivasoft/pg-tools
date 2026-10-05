SELECT d.datname::text AS name, d.oid, pg_get_userbyid(d.datdba)::text AS owner,
       shobj_description(d.oid, 'pg_database') AS comment,
       pg_encoding_to_char(d.encoding)::text AS encoding,
       d.datcollate::text AS collate, d.datctype::text AS ctype,
       t.spcname::text AS tablespace, d.datconnlimit AS conn_limit, d.datallowconn AS allow_conn,
       CASE WHEN has_database_privilege(d.oid, 'CONNECT') THEN pg_size_pretty(pg_database_size(d.oid)) END AS size
FROM pg_database d
JOIN pg_tablespace t ON t.oid = d.dattablespace
WHERE d.oid = $1
