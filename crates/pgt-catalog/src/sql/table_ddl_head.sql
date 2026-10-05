SELECT quote_ident(n.nspname) || '.' || quote_ident(c.relname) AS qname,
       quote_ident(pg_get_userbyid(c.relowner)) AS owner,
       CASE WHEN c.relkind = 'p' THEN pg_get_partkeydef(c.oid) END AS partkey,
       c.relpersistence = 'u' AS unlogged,
       array_to_string(c.reloptions, ', ') AS options,
       quote_ident(t.spcname) AS tablespace,
       quote_literal(obj_description(c.oid, 'pg_class')) AS comment,
       CASE WHEN c.relispartition
            THEN (SELECT quote_ident(pn.nspname) || '.' || quote_ident(pc.relname)
                  FROM pg_inherits i JOIN pg_class pc ON pc.oid = i.inhparent JOIN pg_namespace pn ON pn.oid = pc.relnamespace
                  WHERE i.inhrelid = c.oid) END AS partition_of,
       CASE WHEN c.relispartition THEN pg_get_expr(c.relpartbound, c.oid) END AS partition_bound,
       CASE WHEN NOT c.relispartition
            THEN (SELECT string_agg(quote_ident(pn.nspname) || '.' || quote_ident(pc.relname), ', ' ORDER BY i.inhseqno)
                  FROM pg_inherits i JOIN pg_class pc ON pc.oid = i.inhparent JOIN pg_namespace pn ON pn.oid = pc.relnamespace
                  WHERE i.inhrelid = c.oid) END AS inherits,
       c.relrowsecurity AS rls_enabled,
       c.relforcerowsecurity AS rls_forced
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_tablespace t ON t.oid = c.reltablespace
WHERE c.oid = $1 AND c.relkind IN ('r', 'p')
