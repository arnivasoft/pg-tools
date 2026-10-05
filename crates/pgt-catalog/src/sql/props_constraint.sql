SELECT v.section, v.label, v.value
FROM pg_constraint k
JOIN pg_class c ON c.oid = k.conrelid
JOIN pg_namespace n ON n.oid = c.relnamespace,
LATERAL (VALUES
  (1, 'Genel', 'Ad', k.conname::text),
  (2, 'Genel', 'Tablo', quote_ident(n.nspname) || '.' || quote_ident(c.relname)),
  (3, 'Genel', 'OID', k.oid::text),
  (4, 'Genel', 'Yorum', obj_description(k.oid, 'pg_constraint')),
  (5, 'Tanım', 'Tür', CASE k.contype WHEN 'p' THEN 'primary key' WHEN 'f' THEN 'foreign key' WHEN 'u' THEN 'unique' WHEN 'c' THEN 'check' WHEN 'x' THEN 'exclusion' END),
  (6, 'Tanım', 'Tanım', pg_get_constraintdef(k.oid, true)),
  (7, 'Tanım', 'Ertelenebilir', CASE WHEN k.condeferrable THEN CASE WHEN k.condeferred THEN 'evet (başlangıçta ertelenmiş)' ELSE 'evet' END ELSE 'hayır' END),
  (8, 'Tanım', 'Doğrulanmış', CASE WHEN k.convalidated THEN 'evet' ELSE 'hayır' END)
) AS v(n, section, label, value)
WHERE k.oid = $1
ORDER BY v.n
