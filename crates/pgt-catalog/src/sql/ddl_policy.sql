SELECT 'Policy' AS title,
       quote_ident(p.polname) || ' ON ' || t.qname AS qname,
       format('DROP POLICY IF EXISTS %s ON %s;', quote_ident(p.polname), t.qname) AS drop_stmt,
       format(E'CREATE POLICY %s\n    ON %s\n    AS %s\n    FOR %s\n    TO %s',
              quote_ident(p.polname), t.qname,
              CASE WHEN p.polpermissive THEN 'PERMISSIVE' ELSE 'RESTRICTIVE' END,
              CASE p.polcmd WHEN 'r' THEN 'SELECT' WHEN 'a' THEN 'INSERT' WHEN 'w' THEN 'UPDATE' WHEN 'd' THEN 'DELETE' ELSE 'ALL' END,
              CASE WHEN p.polroles = '{0}' THEN 'PUBLIC'
                   ELSE (SELECT string_agg(quote_ident(r.rolname), ', ' ORDER BY r.rolname) FROM pg_roles r WHERE r.oid = ANY (p.polroles)) END)
         || CASE WHEN p.polqual IS NOT NULL THEN E'\n    USING (' || pg_get_expr(p.polqual, p.polrelid, true) || ')' ELSE '' END
         || CASE WHEN p.polwithcheck IS NOT NULL THEN E'\n    WITH CHECK (' || pg_get_expr(p.polwithcheck, p.polrelid, true) || ')' ELSE '' END
         || E';\n' AS body,
       NULL::text AS extra_stmt,
       NULL::text AS owner_stmt,
       CASE WHEN obj_description(p.oid, 'pg_policy') IS NOT NULL
            THEN format(E'COMMENT ON POLICY %s ON %s\n    IS %L;\n', quote_ident(p.polname), t.qname, obj_description(p.oid, 'pg_policy')) END AS comment_stmt,
       NULL::text AS acl_catalog, NULL::text AS acl_kw, NULL::text AS acl_target
FROM pg_policy p
JOIN pg_class c ON c.oid = p.polrelid
JOIN pg_namespace n ON n.oid = c.relnamespace,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS qname) t
WHERE p.oid = $1
