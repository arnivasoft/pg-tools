SELECT 'Sequence' AS title,
       q.qname,
       'DROP SEQUENCE IF EXISTS ' || q.qname || ';' AS drop_stmt,
       format(E'CREATE SEQUENCE IF NOT EXISTS %s\n    AS %s\n    INCREMENT BY %s\n    MINVALUE %s\n    MAXVALUE %s\n    START WITH %s\n    CACHE %s\n    %s;\n',
              q.qname, format_type(s.seqtypid, NULL), s.seqincrement, s.seqmin, s.seqmax, s.seqstart, s.seqcache,
              CASE WHEN s.seqcycle THEN 'CYCLE' ELSE 'NO CYCLE' END) AS body,
       (SELECT format(E'ALTER SEQUENCE %s\n    OWNED BY %s.%s.%s;\n', q.qname, quote_ident(tn.nspname), quote_ident(tc.relname), quote_ident(ta.attname))
        FROM pg_depend d
        JOIN pg_class tc ON tc.oid = d.refobjid
        JOIN pg_namespace tn ON tn.oid = tc.relnamespace
        JOIN pg_attribute ta ON ta.attrelid = d.refobjid AND ta.attnum = d.refobjsubid
        WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.refclassid = 'pg_class'::regclass AND d.deptype = 'a'
        LIMIT 1) AS extra_stmt,
       format(E'ALTER SEQUENCE %s\n    OWNER TO %s;\n', q.qname, quote_ident(pg_get_userbyid(c.relowner))) AS owner_stmt,
       CASE WHEN obj_description(c.oid, 'pg_class') IS NOT NULL
            THEN format(E'COMMENT ON SEQUENCE %s\n    IS %L;\n', q.qname, obj_description(c.oid, 'pg_class')) END AS comment_stmt,
       'relation' AS acl_catalog, 'SEQUENCE' AS acl_kw, q.qname AS acl_target
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_sequence s ON s.seqrelid = c.oid,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS qname) q
WHERE c.oid = $1
