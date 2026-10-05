SELECT 'Tablespace' AS title,
       quote_ident(t.spcname) AS qname,
       'DROP TABLESPACE IF EXISTS ' || quote_ident(t.spcname) || ';' AS drop_stmt,
       CASE WHEN t.spcname LIKE 'pg\_%' THEN E'-- Yerleşik tablespace; oluşturulamaz.\n'
            ELSE format(E'CREATE TABLESPACE %s\n    OWNER %s\n    LOCATION %L;\n', quote_ident(t.spcname), quote_ident(pg_get_userbyid(t.spcowner)), pg_tablespace_location(t.oid)) END AS body,
       CASE WHEN t.spcoptions IS NOT NULL THEN format(E'ALTER TABLESPACE %s SET (%s);\n', quote_ident(t.spcname), array_to_string(t.spcoptions, ', ')) END AS extra_stmt,
       NULL::text AS owner_stmt,
       CASE WHEN shobj_description(t.oid, 'pg_tablespace') IS NOT NULL
            THEN format(E'COMMENT ON TABLESPACE %s\n    IS %L;\n', quote_ident(t.spcname), shobj_description(t.oid, 'pg_tablespace')) END AS comment_stmt,
       'tablespace' AS acl_catalog, 'TABLESPACE' AS acl_kw, quote_ident(t.spcname) AS acl_target
FROM pg_tablespace t
WHERE t.oid = $1
