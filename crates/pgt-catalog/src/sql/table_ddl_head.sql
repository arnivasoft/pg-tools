SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS qname,
       quote_ident(pg_get_userbyid(c.relowner)) AS owner,
       CASE WHEN c.relkind = 'p' THEN pg_get_partkeydef(c.oid) END AS partkey,
       c.relpersistence = 'u' AS unlogged,
       array_to_string(c.reloptions, ', ') AS options,
       quote_ident(t.spcname) AS tablespace,
       quote_literal(obj_description(c.oid, 'pg_class')) AS comment
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_tablespace t ON t.oid = c.reltablespace
WHERE c.oid = $1 AND c.relkind IN ('r', 'p')
