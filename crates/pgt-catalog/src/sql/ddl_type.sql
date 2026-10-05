SELECT 'Type' AS title,
       q.qname,
       'DROP TYPE IF EXISTS ' || q.qname || ';' AS drop_stmt,
       CASE t.typtype
         WHEN 'e' THEN format(E'CREATE TYPE %s AS ENUM (\n%s\n);\n', q.qname,
                              (SELECT string_agg('    ' || quote_literal(e.enumlabel), E',\n' ORDER BY e.enumsortorder) FROM pg_enum e WHERE e.enumtypid = t.oid))
         WHEN 'c' THEN format(E'CREATE TYPE %s AS (\n%s\n);\n', q.qname,
                              (SELECT string_agg('    ' || quote_ident(a.attname) || ' ' || format_type(a.atttypid, a.atttypmod), E',\n' ORDER BY a.attnum)
                               FROM pg_attribute a WHERE a.attrelid = t.typrelid AND a.attnum > 0 AND NOT a.attisdropped))
         WHEN 'r' THEN format(E'CREATE TYPE %s AS RANGE (\n    SUBTYPE = %s\n);\n', q.qname,
                              (SELECT format_type(r.rngsubtype, NULL) FROM pg_range r WHERE r.rngtypid = t.oid))
         ELSE E'-- Temel (base) tiplerin DDL\'i desteklenmez.\n'
       END AS body,
       NULL::text AS extra_stmt,
       format(E'ALTER TYPE %s\n    OWNER TO %s;\n', q.qname, quote_ident(pg_get_userbyid(t.typowner))) AS owner_stmt,
       CASE WHEN obj_description(t.oid, 'pg_type') IS NOT NULL
            THEN format(E'COMMENT ON TYPE %s\n    IS %L;\n', q.qname, obj_description(t.oid, 'pg_type')) END AS comment_stmt,
       'type' AS acl_catalog, 'TYPE' AS acl_kw, q.qname AS acl_target
FROM pg_type t
JOIN pg_namespace n ON n.oid = t.typnamespace,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(t.typname) AS qname) q
WHERE t.oid = $1 AND t.typtype IN ('e', 'c', 'r', 'b')
