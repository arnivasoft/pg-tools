SELECT 'Extension' AS title,
       quote_ident(e.extname) AS qname,
       'DROP EXTENSION IF EXISTS ' || quote_ident(e.extname) || ';' AS drop_stmt,
       format(E'CREATE EXTENSION IF NOT EXISTS %s\n    SCHEMA %s\n    VERSION %s;\n', quote_ident(e.extname), quote_ident(n.nspname), quote_ident(e.extversion)) AS body,
       NULL::text AS extra_stmt,
       NULL::text AS owner_stmt,
       CASE WHEN obj_description(e.oid, 'pg_extension') IS NOT NULL
            THEN format(E'COMMENT ON EXTENSION %s\n    IS %L;\n', quote_ident(e.extname), obj_description(e.oid, 'pg_extension')) END AS comment_stmt,
       NULL::text AS acl_catalog, NULL::text AS acl_kw, NULL::text AS acl_target
FROM pg_extension e
JOIN pg_namespace n ON n.oid = e.extnamespace
WHERE e.oid = $1
