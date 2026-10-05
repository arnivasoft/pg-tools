SELECT 'Domain' AS title,
       q.qname,
       'DROP DOMAIN IF EXISTS ' || q.qname || ';' AS drop_stmt,
       'CREATE DOMAIN ' || q.qname || ' AS ' || format_type(t.typbasetype, t.typtypmod)
         || CASE WHEN t.typcollation <> 0 AND t.typcollation <> bt.typcollation
                 THEN ' COLLATE ' || (SELECT quote_ident(cn.nspname) || '.' || quote_ident(co.collname) FROM pg_collation co JOIN pg_namespace cn ON cn.oid = co.collnamespace WHERE co.oid = t.typcollation)
                 ELSE '' END
         || CASE WHEN t.typdefault IS NOT NULL THEN E'\n    DEFAULT ' || t.typdefault ELSE '' END
         || CASE WHEN t.typnotnull THEN E'\n    NOT NULL' ELSE '' END
         || coalesce((SELECT string_agg(E'\n    CONSTRAINT ' || quote_ident(k.conname) || ' ' || pg_get_constraintdef(k.oid, true), '' ORDER BY k.conname COLLATE "C")
                      FROM pg_constraint k WHERE k.contypid = t.oid AND k.contype = 'c'), '')
         || E';\n' AS body,
       NULL::text AS extra_stmt,
       format(E'ALTER DOMAIN %s\n    OWNER TO %s;\n', q.qname, quote_ident(pg_get_userbyid(t.typowner))) AS owner_stmt,
       CASE WHEN obj_description(t.oid, 'pg_type') IS NOT NULL
            THEN format(E'COMMENT ON DOMAIN %s\n    IS %L;\n', q.qname, obj_description(t.oid, 'pg_type')) END AS comment_stmt,
       'type' AS acl_catalog, 'DOMAIN' AS acl_kw, q.qname AS acl_target
FROM pg_type t
JOIN pg_type bt ON bt.oid = t.typbasetype
JOIN pg_namespace n ON n.oid = t.typnamespace,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(t.typname) AS qname) q
WHERE t.oid = $1 AND t.typtype = 'd'
