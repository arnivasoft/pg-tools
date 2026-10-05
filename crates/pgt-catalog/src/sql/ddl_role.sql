SELECT 'Role' AS title,
       quote_ident(r.rolname) AS qname,
       'DROP ROLE IF EXISTS ' || quote_ident(r.rolname) || ';' AS drop_stmt,
       format(E'CREATE ROLE %s WITH\n    %s\n    %s\n    %s\n    %s\n    %s\n    %s\n    %s\n    CONNECTION LIMIT %s%s;\n',
              quote_ident(r.rolname),
              CASE WHEN r.rolcanlogin THEN 'LOGIN' ELSE 'NOLOGIN' END,
              CASE WHEN r.rolsuper THEN 'SUPERUSER' ELSE 'NOSUPERUSER' END,
              CASE WHEN r.rolinherit THEN 'INHERIT' ELSE 'NOINHERIT' END,
              CASE WHEN r.rolcreatedb THEN 'CREATEDB' ELSE 'NOCREATEDB' END,
              CASE WHEN r.rolcreaterole THEN 'CREATEROLE' ELSE 'NOCREATEROLE' END,
              CASE WHEN r.rolreplication THEN 'REPLICATION' ELSE 'NOREPLICATION' END,
              CASE WHEN r.rolbypassrls THEN 'BYPASSRLS' ELSE 'NOBYPASSRLS' END,
              r.rolconnlimit,
              CASE WHEN r.rolvaliduntil IS NOT NULL THEN format(E'\n    VALID UNTIL %L', to_char(r.rolvaliduntil, 'YYYY-MM-DD HH24:MI:SS')) ELSE '' END) AS body,
       (SELECT string_agg(format('GRANT %s TO %s%s;', quote_ident(g.rolname), quote_ident(r.rolname), CASE WHEN m.admin_option THEN ' WITH ADMIN OPTION' ELSE '' END) || E'\n', '' ORDER BY g.rolname::text COLLATE "C")
        FROM pg_auth_members m JOIN pg_roles g ON g.oid = m.roleid WHERE m.member = r.oid) AS extra_stmt,
       NULL::text AS owner_stmt,
       CASE WHEN shobj_description(r.oid, 'pg_authid') IS NOT NULL
            THEN format(E'COMMENT ON ROLE %s\n    IS %L;\n', quote_ident(r.rolname), shobj_description(r.oid, 'pg_authid')) END AS comment_stmt,
       NULL::text AS acl_catalog, NULL::text AS acl_kw, NULL::text AS acl_target
FROM pg_roles r
WHERE r.oid = $1
