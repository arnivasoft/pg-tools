SELECT v.section, v.label, v.value
FROM pg_tablespace t,
LATERAL (VALUES
  (1, 'Genel', 'Ad', t.spcname::text),
  (2, 'Genel', 'OID', t.oid::text),
  (3, 'Genel', 'Sahip', pg_get_userbyid(t.spcowner)::text),
  (4, 'Genel', 'Yorum', shobj_description(t.oid, 'pg_tablespace')),
  (5, 'Tanım', 'Konum', NULLIF(pg_tablespace_location(t.oid), '')),
  (6, 'Tanım', 'Seçenekler', array_to_string(t.spcoptions, ', ')),
  (7, 'İstatistik', 'Boyut', pg_size_pretty(pg_tablespace_size(t.oid)))
) AS v(n, section, label, value)
WHERE t.oid = $1
ORDER BY v.n
