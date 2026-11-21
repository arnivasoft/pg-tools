-- PG 15+: kolon listesi (prattrs) ve satır süzgeci (prqual).
SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS qname,
       CASE WHEN r.prattrs IS NOT NULL
            THEN (SELECT string_agg(quote_ident(a.attname), ', ' ORDER BY a.attnum)
                  FROM pg_attribute a WHERE a.attrelid = r.prrelid AND a.attnum = ANY (r.prattrs::int2[])) END AS columns,
       pg_get_expr(r.prqual, r.prrelid, true) AS qual
FROM pg_publication_rel r
JOIN pg_class c ON c.oid = r.prrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE r.prpubid = $1
ORDER BY n.nspname::text COLLATE "C", c.relname::text COLLATE "C"
