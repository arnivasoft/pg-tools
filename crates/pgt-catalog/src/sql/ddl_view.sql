SELECT CASE c.relkind WHEN 'm' THEN 'Materialized View' ELSE 'View' END AS title,
       q.qname,
       CASE c.relkind WHEN 'm' THEN 'DROP MATERIALIZED VIEW IF EXISTS ' ELSE 'DROP VIEW IF EXISTS ' END || q.qname || ';' AS drop_stmt,
       CASE c.relkind
         WHEN 'm' THEN
           'CREATE MATERIALIZED VIEW IF NOT EXISTS ' || q.qname
           || CASE WHEN c.reloptions IS NOT NULL THEN E'\nWITH (' || array_to_string(c.reloptions, ', ') || ')' ELSE '' END
           || E'\nAS\n' || rtrim(pg_get_viewdef(c.oid, true), E';\n ')
           || CASE WHEN c.relispopulated THEN E'\nWITH DATA;\n' ELSE E'\nWITH NO DATA;\n' END
           || coalesce((SELECT E'\n' || string_agg(pg_get_indexdef(x.indexrelid) || E';\n', E'\n' ORDER BY ic.relname::text COLLATE "C")
                        FROM pg_index x JOIN pg_class ic ON ic.oid = x.indexrelid WHERE x.indrelid = c.oid), '')
         ELSE
           'CREATE OR REPLACE VIEW ' || q.qname
           || CASE WHEN c.reloptions IS NOT NULL THEN E'\nWITH (' || array_to_string(c.reloptions, ', ') || ')' ELSE '' END
           || E'\nAS\n' || pg_get_viewdef(c.oid, true) || E'\n'
       END AS body,
       NULL::text AS extra_stmt,
       format(E'ALTER %s %s\n    OWNER TO %s;\n', CASE c.relkind WHEN 'm' THEN 'MATERIALIZED VIEW' ELSE 'TABLE' END, q.qname, quote_ident(pg_get_userbyid(c.relowner))) AS owner_stmt,
       CASE WHEN obj_description(c.oid, 'pg_class') IS NOT NULL
            THEN format(E'COMMENT ON %s %s\n    IS %L;\n', CASE c.relkind WHEN 'm' THEN 'MATERIALIZED VIEW' ELSE 'VIEW' END, q.qname, obj_description(c.oid, 'pg_class')) END AS comment_stmt,
       'relation' AS acl_catalog, 'TABLE' AS acl_kw, q.qname AS acl_target
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS qname) q
WHERE c.oid = $1 AND c.relkind IN ('v', 'm')
