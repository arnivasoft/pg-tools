SELECT format(E'CREATE POLICY %s\n    ON %s\n    AS %s\n    FOR %s\n    TO %s',
              quote_ident(p.polname), quote_ident(n.nspname) || '.' || quote_ident(c.relname),
              CASE WHEN p.polpermissive THEN 'PERMISSIVE' ELSE 'RESTRICTIVE' END,
              CASE p.polcmd WHEN 'r' THEN 'SELECT' WHEN 'a' THEN 'INSERT' WHEN 'w' THEN 'UPDATE' WHEN 'd' THEN 'DELETE' ELSE 'ALL' END,
              CASE WHEN p.polroles = '{0}' THEN 'PUBLIC'
                   ELSE (SELECT string_agg(quote_ident(r.rolname), ', ' ORDER BY r.rolname) FROM pg_roles r WHERE r.oid = ANY (p.polroles)) END)
         || CASE WHEN p.polqual IS NOT NULL THEN E'\n    USING (' || pg_get_expr(p.polqual, p.polrelid, true) || ')' ELSE '' END
         || CASE WHEN p.polwithcheck IS NOT NULL THEN E'\n    WITH CHECK (' || pg_get_expr(p.polwithcheck, p.polrelid, true) || ')' ELSE '' END
         || ';' AS def,
       quote_ident(p.polname) AS name,
       quote_literal(obj_description(p.oid, 'pg_policy')) AS comment
FROM pg_policy p
JOIN pg_class c ON c.oid = p.polrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE p.polrelid = $1
ORDER BY p.polname COLLATE "C"
