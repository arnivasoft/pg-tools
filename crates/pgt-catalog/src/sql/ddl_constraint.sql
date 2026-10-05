SELECT 'Constraint' AS title,
       quote_ident(k.conname) || ' ON ' || t.qname AS qname,
       format('ALTER TABLE IF EXISTS %s DROP CONSTRAINT IF EXISTS %s;', t.qname, quote_ident(k.conname)) AS drop_stmt,
       format(E'ALTER TABLE IF EXISTS %s\n    ADD CONSTRAINT %s %s;\n', t.qname, quote_ident(k.conname), pg_get_constraintdef(k.oid, true)) AS body,
       NULL::text AS extra_stmt,
       NULL::text AS owner_stmt,
       CASE WHEN obj_description(k.oid, 'pg_constraint') IS NOT NULL
            THEN format(E'COMMENT ON CONSTRAINT %s ON %s\n    IS %L;\n', quote_ident(k.conname), t.qname, obj_description(k.oid, 'pg_constraint')) END AS comment_stmt,
       NULL::text AS acl_catalog, NULL::text AS acl_kw, NULL::text AS acl_target
FROM pg_constraint k
JOIN pg_class c ON c.oid = k.conrelid
JOIN pg_namespace n ON n.oid = c.relnamespace,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS qname) t
WHERE k.oid = $1
