SELECT v.section, v.label, v.value
FROM pg_index x
JOIN pg_class i ON i.oid = x.indexrelid
JOIN pg_class t ON t.oid = x.indrelid
JOIN pg_namespace n ON n.oid = i.relnamespace
JOIN pg_am am ON am.oid = i.relam
LEFT JOIN pg_tablespace ts ON ts.oid = i.reltablespace,
LATERAL (VALUES
  (1, 'Genel', 'Ad', i.relname::text),
  (2, 'Genel', 'Şema', n.nspname::text),
  (3, 'Genel', 'Tablo', t.relname::text),
  (4, 'Genel', 'OID', i.oid::text),
  (5, 'Genel', 'Yorum', obj_description(i.oid, 'pg_class')),
  (6, 'Tanım', 'Erişim yöntemi', am.amname::text),
  (7, 'Tanım', 'Tanım', pg_get_indexdef(i.oid)),
  (8, 'Tanım', 'Benzersiz', CASE WHEN x.indisunique THEN 'evet' ELSE 'hayır' END),
  (9, 'Tanım', 'Birincil anahtar', CASE WHEN x.indisprimary THEN 'evet' ELSE 'hayır' END),
  (10, 'Tanım', 'Geçerli', CASE WHEN x.indisvalid THEN 'evet' ELSE 'hayır' END),
  (11, 'Tanım', 'Tablespace', COALESCE(ts.spcname::text, 'pg_default')),
  (12, 'Tanım', 'Seçenekler', array_to_string(i.reloptions, ', ')),
  (13, 'İstatistik', 'Boyut', pg_size_pretty(pg_relation_size(i.oid)))
) AS v(n, section, label, value)
WHERE x.indexrelid = $1
ORDER BY v.n
