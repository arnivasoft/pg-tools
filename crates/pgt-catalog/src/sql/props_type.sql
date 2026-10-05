SELECT v.section, v.label, v.value
FROM pg_type t
JOIN pg_namespace n ON n.oid = t.typnamespace,
LATERAL (VALUES
  (1, 'Genel', 'Ad', t.typname::text),
  (2, 'Genel', 'Şema', n.nspname::text),
  (3, 'Genel', 'OID', t.oid::text),
  (4, 'Genel', 'Sahip', pg_get_userbyid(t.typowner)::text),
  (5, 'Genel', 'Yorum', obj_description(t.oid, 'pg_type')),
  (6, 'Tanım', 'Tür', CASE t.typtype WHEN 'e' THEN 'enum' WHEN 'c' THEN 'composite' WHEN 'r' THEN 'range' WHEN 'd' THEN 'domain' ELSE 'base' END),
  (7, 'Tanım', 'Etiketler', CASE WHEN t.typtype = 'e' THEN (SELECT string_agg(quote_literal(e.enumlabel), ', ' ORDER BY e.enumsortorder) FROM pg_enum e WHERE e.enumtypid = t.oid) END),
  (8, 'Tanım', 'Alanlar', CASE WHEN t.typtype = 'c' THEN (SELECT string_agg(quote_ident(a.attname) || ' ' || format_type(a.atttypid, a.atttypmod), ', ' ORDER BY a.attnum) FROM pg_attribute a WHERE a.attrelid = t.typrelid AND a.attnum > 0 AND NOT a.attisdropped) END),
  (9, 'Tanım', 'Alt tip', CASE WHEN t.typtype = 'r' THEN (SELECT format_type(r.rngsubtype, NULL) FROM pg_range r WHERE r.rngtypid = t.oid) END),
  (10, 'Tanım', 'Taban tip', CASE WHEN t.typtype = 'd' THEN format_type(t.typbasetype, t.typtypmod) END),
  (11, 'Tanım', 'Boş olamaz', CASE WHEN t.typtype = 'd' THEN CASE WHEN t.typnotnull THEN 'evet' ELSE 'hayır' END END),
  (12, 'Tanım', 'Varsayılan', t.typdefault)
) AS v(n, section, label, value)
WHERE t.oid = $1
ORDER BY v.n
