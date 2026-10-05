SELECT 'Trigger' AS title,
       quote_ident(g.tgname) || ' ON ' || t.qname AS qname,
       format('DROP TRIGGER IF EXISTS %s ON %s;', quote_ident(g.tgname), t.qname) AS drop_stmt,
       pg_get_triggerdef(g.oid, true) || E';\n' AS body,
       CASE g.tgenabled
         WHEN 'D' THEN format(E'ALTER TABLE IF EXISTS %s DISABLE TRIGGER %s;\n', t.qname, quote_ident(g.tgname))
         WHEN 'R' THEN format(E'ALTER TABLE IF EXISTS %s ENABLE REPLICA TRIGGER %s;\n', t.qname, quote_ident(g.tgname))
         WHEN 'A' THEN format(E'ALTER TABLE IF EXISTS %s ENABLE ALWAYS TRIGGER %s;\n', t.qname, quote_ident(g.tgname))
       END AS extra_stmt,
       NULL::text AS owner_stmt,
       CASE WHEN obj_description(g.oid, 'pg_trigger') IS NOT NULL
            THEN format(E'COMMENT ON TRIGGER %s ON %s\n    IS %L;\n', quote_ident(g.tgname), t.qname, obj_description(g.oid, 'pg_trigger')) END AS comment_stmt,
       NULL::text AS acl_catalog, NULL::text AS acl_kw, NULL::text AS acl_target
FROM pg_trigger g
JOIN pg_class c ON c.oid = g.tgrelid
JOIN pg_namespace n ON n.oid = c.relnamespace,
LATERAL (SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS qname) t
WHERE g.oid = $1
