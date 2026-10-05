SELECT v.section, v.label, v.value
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_sequence s ON s.seqrelid = c.oid
LEFT JOIN LATERAL (
  SELECT quote_ident(tn.nspname) || '.' || quote_ident(tc.relname) || '.' || quote_ident(ta.attname) AS owner_col
  FROM pg_depend d
  JOIN pg_class tc ON tc.oid = d.refobjid
  JOIN pg_namespace tn ON tn.oid = tc.relnamespace
  JOIN pg_attribute ta ON ta.attrelid = d.refobjid AND ta.attnum = d.refobjsubid
  WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.refclassid = 'pg_class'::regclass AND d.deptype = 'a'
  LIMIT 1
) own ON true,
LATERAL (VALUES
  (1, 'Genel', 'Ad', c.relname::text),
  (2, 'Genel', 'Şema', n.nspname::text),
  (3, 'Genel', 'OID', c.oid::text),
  (4, 'Genel', 'Sahip', pg_get_userbyid(c.relowner)::text),
  (5, 'Genel', 'Yorum', obj_description(c.oid, 'pg_class')),
  (6, 'Tanım', 'Veri tipi', format_type(s.seqtypid, NULL)),
  (7, 'Tanım', 'Başlangıç', s.seqstart::text),
  (8, 'Tanım', 'Artış', s.seqincrement::text),
  (9, 'Tanım', 'En küçük', s.seqmin::text),
  (10, 'Tanım', 'En büyük', s.seqmax::text),
  (11, 'Tanım', 'Önbellek', s.seqcache::text),
  (12, 'Tanım', 'Döngü', CASE WHEN s.seqcycle THEN 'evet' ELSE 'hayır' END),
  (13, 'Tanım', 'Sahibi olan kolon', own.owner_col)
) AS v(n, section, label, value)
WHERE c.oid = $1
ORDER BY v.n
