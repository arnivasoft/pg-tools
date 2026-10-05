SELECT quote_ident(n.nspname) AS qname, quote_ident(pg_get_userbyid(n.nspowner)) AS owner,
       quote_literal(obj_description(n.oid, 'pg_namespace')) AS comment
FROM pg_namespace n
WHERE n.oid = $1
