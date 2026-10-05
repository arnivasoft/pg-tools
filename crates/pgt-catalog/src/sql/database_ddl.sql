SELECT quote_ident(d.datname) AS qname, quote_ident(pg_get_userbyid(d.datdba)) AS owner,
       pg_encoding_to_char(d.encoding)::text AS encoding, d.datcollate::text AS collate, d.datctype::text AS ctype,
       quote_ident(t.spcname) AS tablespace, d.datconnlimit AS conn_limit, d.datistemplate AS is_template,
       quote_literal(shobj_description(d.oid, 'pg_database')) AS comment
FROM pg_database d
JOIN pg_tablespace t ON t.oid = d.dattablespace
WHERE d.oid = $1
