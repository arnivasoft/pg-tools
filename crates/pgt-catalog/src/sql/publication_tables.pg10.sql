SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS qname, NULL::text AS columns, NULL::text AS qual
FROM pg_publication_rel r
JOIN pg_class c ON c.oid = r.prrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE r.prpubid = $1
ORDER BY n.nspname::text COLLATE "C", c.relname::text COLLATE "C"
