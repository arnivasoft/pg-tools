SELECT 'Index' AS title,
       q.qname,
       'DROP INDEX IF EXISTS ' || q.qname || ';' AS drop_stmt,
       pg_get_indexdef(i.oid) || E';\n' AS body,
       NULL::text AS extra_stmt,
       NULL::text AS owner_stmt,
       CASE WHEN obj_description(i.oid, 'pg_class') IS NOT NULL
            THEN format(E'COMMENT ON INDEX %s\n    IS %L;\n', q.qname, obj_description(i.oid, 'pg_class')) END AS comment_stmt,
       NULL::text AS acl_catalog, NULL::text AS acl_kw, NULL::text AS acl_target
FROM pg_class i
JOIN pg_namespace n ON n.oid = i.relnamespace,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(i.relname) AS qname) q
WHERE i.oid = $1 AND i.relkind IN ('i', 'I')
