SELECT 'Function' AS title,
       q.target AS qname,
       format('DROP FUNCTION IF EXISTS %s;', q.target) AS drop_stmt,
       rtrim(pg_get_functiondef(p.oid), E'\n') || E';\n' AS body,
       NULL::text AS extra_stmt,
       format(E'ALTER FUNCTION %s\n    OWNER TO %s;\n', q.target, quote_ident(pg_get_userbyid(p.proowner))) AS owner_stmt,
       CASE WHEN obj_description(p.oid, 'pg_proc') IS NOT NULL
            THEN format(E'COMMENT ON FUNCTION %s\n    IS %L;\n', q.target, obj_description(p.oid, 'pg_proc')) END AS comment_stmt,
       'proc' AS acl_catalog, 'FUNCTION' AS acl_kw, q.target AS acl_target
FROM pg_proc p
JOIN pg_namespace n ON n.oid = p.pronamespace,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(p.proname) || '(' || pg_get_function_identity_arguments(p.oid) || ')' AS target) q
WHERE p.oid = $1
