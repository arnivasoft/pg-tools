SELECT v.section, v.label, v.value
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace,
LATERAL (VALUES
  (1, 'Genel', 'Ad', c.relname::text),
  (2, 'Genel', 'Şema', n.nspname::text),
  (3, 'Genel', 'OID', c.oid::text),
  (4, 'Genel', 'Sahip', pg_get_userbyid(c.relowner)::text),
  (5, 'Genel', 'Yorum', obj_description(c.oid, 'pg_class')),
  (6, 'Tanım', 'Tür', CASE c.relkind WHEN 'm' THEN 'Materialized view' ELSE 'View' END),
  (7, 'Tanım', 'Seçenekler', array_to_string(c.reloptions, ', ')),
  (8, 'Tanım', 'Doldurulmuş', CASE WHEN c.relkind = 'm' THEN CASE WHEN c.relispopulated THEN 'evet' ELSE 'hayır' END END),
  (9, 'İstatistik', 'Boyut', CASE WHEN c.relkind = 'm' THEN pg_size_pretty(pg_total_relation_size(c.oid)) END)
) AS v(n, section, label, value)
WHERE c.oid = $1 AND c.relkind IN ('v', 'm')
ORDER BY v.n
