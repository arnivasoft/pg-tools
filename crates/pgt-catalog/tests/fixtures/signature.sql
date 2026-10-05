-- Gidiş-dönüş testi için katalog imzası: OID içermeyen, iki veritabanında karşılaştırılabilir satırlar (kind, key, def).
-- Kapsam: app ve extra şemaları; extension üyeleri hariç.
WITH ns AS (
    SELECT oid, nspname::text AS nspname FROM pg_namespace WHERE nspname IN ('app', 'extra')
),
rel AS (
    SELECT c.oid, c.relkind, c.relpersistence, c.reloptions, c.relrowsecurity, c.relforcerowsecurity, c.relowner, c.relacl,
           c.relispartition, c.relpartbound, c.relispopulated, n.nspname || '.' || c.relname AS qname
    FROM pg_class c
    JOIN ns n ON n.oid = c.relnamespace
    WHERE c.relkind IN ('r', 'p', 'v', 'm', 'S')
      AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'e')
)
SELECT 'schema' AS kind, n.nspname AS key,
       concat_ws(' | ', pg_get_userbyid(s.nspowner), coalesce(s.nspacl::text, ''), coalesce(obj_description(s.oid, 'pg_namespace'), '')) AS def
FROM ns n JOIN pg_namespace s ON s.oid = n.oid
UNION ALL
SELECT 'relation', r.qname,
       concat_ws(' | ', r.relkind::text, r.relpersistence::text, coalesce(array_to_string(r.reloptions, ','), ''),
                 r.relrowsecurity, r.relforcerowsecurity, pg_get_userbyid(r.relowner), coalesce(r.relacl::text, ''),
                 coalesce(obj_description(r.oid, 'pg_class'), ''),
                 CASE WHEN r.relkind = 'p' THEN pg_get_partkeydef(r.oid) ELSE '' END,
                 CASE WHEN r.relispartition THEN pg_get_expr(r.relpartbound, r.oid) ELSE '' END,
                 coalesce((SELECT string_agg(pn.nspname || '.' || pc.relname, ',' ORDER BY i.inhseqno)
                           FROM pg_inherits i JOIN pg_class pc ON pc.oid = i.inhparent JOIN pg_namespace pn ON pn.oid = pc.relnamespace
                           WHERE i.inhrelid = r.oid), ''),
                 CASE WHEN r.relkind IN ('v', 'm') THEN pg_get_viewdef(r.oid, true) ELSE '' END,
                 CASE WHEN r.relkind = 'm' THEN r.relispopulated::text ELSE '' END)
FROM rel r
UNION ALL
SELECT 'column', r.qname || '.' || a.attname,
       concat_ws(' | ', format_type(a.atttypid, a.atttypmod), a.attnotnull, coalesce(pg_get_expr(d.adbin, d.adrelid), ''),
                 a.attidentity::text, a.attgenerated::text,
                 coalesce((SELECT co.collname::text FROM pg_collation co WHERE co.oid = a.attcollation), ''),
                 coalesce(a.attacl::text, ''), coalesce(col_description(a.attrelid, a.attnum), ''))
FROM rel r
JOIN pg_attribute a ON a.attrelid = r.oid AND a.attnum > 0 AND NOT a.attisdropped
LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
WHERE r.relkind IN ('r', 'p', 'v', 'm')
UNION ALL
SELECT 'constraint', r.qname || '.' || k.conname, pg_get_constraintdef(k.oid, true) || ' | ' || k.convalidated
FROM rel r JOIN pg_constraint k ON k.conrelid = r.oid AND k.contype IN ('p', 'u', 'c', 'x', 'f')
UNION ALL
SELECT 'index', n.nspname || '.' || ic.relname, pg_get_indexdef(x.indexrelid) || ' | ' || coalesce(obj_description(ic.oid, 'pg_class'), '')
FROM pg_index x
JOIN pg_class ic ON ic.oid = x.indexrelid
JOIN ns n ON n.oid = ic.relnamespace
UNION ALL
SELECT 'trigger', r.qname || '.' || t.tgname, pg_get_triggerdef(t.oid, true) || ' | ' || t.tgenabled::text || ' | ' || coalesce(obj_description(t.oid, 'pg_trigger'), '')
FROM rel r JOIN pg_trigger t ON t.tgrelid = r.oid AND NOT t.tgisinternal
UNION ALL
SELECT 'policy', r.qname || '.' || p.polname,
       concat_ws(' | ', p.polcmd::text, p.polpermissive, p.polroles::regrole[]::text,
                 coalesce(pg_get_expr(p.polqual, p.polrelid, true), ''), coalesce(pg_get_expr(p.polwithcheck, p.polrelid, true), ''))
FROM rel r JOIN pg_policy p ON p.polrelid = r.oid
UNION ALL
SELECT 'sequence', r.qname, concat_ws(' | ', s.seqtypid::regtype::text, s.seqstart, s.seqincrement, s.seqmin, s.seqmax, s.seqcache, s.seqcycle)
FROM rel r JOIN pg_sequence s ON s.seqrelid = r.oid
UNION ALL
SELECT 'function', n.nspname || '.' || p.proname || '(' || pg_get_function_identity_arguments(p.oid) || ')',
       concat_ws(' | ', pg_get_functiondef(p.oid), coalesce(p.proacl::text, ''), pg_get_userbyid(p.proowner), coalesce(obj_description(p.oid, 'pg_proc'), ''))
FROM pg_proc p
JOIN ns n ON n.oid = p.pronamespace
WHERE NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype IN ('e', 'i'))
UNION ALL
SELECT 'type', n.nspname || '.' || t.typname,
       concat_ws(' | ', t.typtype::text, pg_get_userbyid(t.typowner), coalesce(t.typacl::text, ''), coalesce(obj_description(t.oid, 'pg_type'), ''),
                 CASE WHEN t.typtype = 'e' THEN (SELECT string_agg(e.enumlabel, ',' ORDER BY e.enumsortorder) FROM pg_enum e WHERE e.enumtypid = t.oid) END,
                 CASE WHEN t.typtype = 'c' THEN (SELECT string_agg(a.attname || ' ' || format_type(a.atttypid, a.atttypmod), ',' ORDER BY a.attnum)
                                                  FROM pg_attribute a WHERE a.attrelid = t.typrelid AND a.attnum > 0 AND NOT a.attisdropped) END,
                 CASE WHEN t.typtype = 'r' THEN (SELECT format_type(r.rngsubtype, NULL) FROM pg_range r WHERE r.rngtypid = t.oid) END,
                 CASE WHEN t.typtype = 'd' THEN format_type(t.typbasetype, t.typtypmod) || ' ' || t.typnotnull || ' ' || coalesce(t.typdefault, '')
                        || ' ' || coalesce((SELECT string_agg(k.conname || ':' || pg_get_constraintdef(k.oid, true), ',' ORDER BY k.conname) FROM pg_constraint k WHERE k.contypid = t.oid), '') END)
FROM pg_type t
JOIN ns n ON n.oid = t.typnamespace
WHERE t.typtype IN ('e', 'c', 'r', 'd')
  AND (t.typtype <> 'c' OR EXISTS (SELECT 1 FROM pg_class c WHERE c.oid = t.typrelid AND c.relkind = 'c'))
  AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_type'::regclass AND d.objid = t.oid AND d.deptype = 'e')
ORDER BY kind, key
