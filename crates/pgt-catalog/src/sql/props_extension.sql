SELECT v.section, v.label, v.value
FROM pg_extension e
JOIN pg_namespace n ON n.oid = e.extnamespace,
LATERAL (VALUES
  (1, 'Genel', 'Ad', e.extname::text),
  (2, 'Genel', 'OID', e.oid::text),
  (3, 'Genel', 'Sahip', pg_get_userbyid(e.extowner)::text),
  (4, 'Genel', 'Yorum', obj_description(e.oid, 'pg_extension')),
  (5, 'Tanım', 'Sürüm', e.extversion),
  (6, 'Tanım', 'Şema', n.nspname::text),
  (7, 'Tanım', 'Yeniden konumlandırılabilir', CASE WHEN e.extrelocatable THEN 'evet' ELSE 'hayır' END)
) AS v(n, section, label, value)
WHERE e.oid = $1
ORDER BY v.n
