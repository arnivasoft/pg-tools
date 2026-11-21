SELECT c.oid, c.relname::text AS name, n.nspname::text AS schema,
       quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS qname,
       pg_get_userbyid(c.relowner)::text AS owner, quote_ident(pg_get_userbyid(c.relowner)) AS qowner,
       obj_description(c.oid, 'pg_class') AS comment,
       s.srvname::text AS server, quote_ident(s.srvname) AS qserver,
       COALESCE(ft.ftoptions, '{}') AS options,
       ARRAY(SELECT quote_ident(o.k) FROM pg_options_to_table(ft.ftoptions) WITH ORDINALITY o(k, v, n) ORDER BY o.n) AS option_keys,
       (SELECT count(*) FROM pg_attribute a WHERE a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped)::int4 AS column_count,
       (SELECT string_agg(quote_ident(pn.nspname) || '.' || quote_ident(pc.relname), ', ' ORDER BY i.inhseqno)
        FROM pg_inherits i JOIN pg_class pc ON pc.oid = i.inhparent JOIN pg_namespace pn ON pn.oid = pc.relnamespace
        WHERE i.inhrelid = c.oid) AS inherits
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_foreign_table ft ON ft.ftrelid = c.oid
JOIN pg_foreign_server s ON s.oid = ft.ftserver
WHERE c.oid = $1 AND c.relkind = 'f'
