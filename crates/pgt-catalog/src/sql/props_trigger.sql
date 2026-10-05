SELECT v.section, v.label, v.value
FROM pg_trigger g
JOIN pg_class c ON c.oid = g.tgrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_proc p ON p.oid = g.tgfoid
JOIN pg_namespace pn ON pn.oid = p.pronamespace,
LATERAL (VALUES
  (1, 'Genel', 'Ad', g.tgname::text),
  (2, 'Genel', 'Tablo', quote_ident(n.nspname) || '.' || quote_ident(c.relname)),
  (3, 'Genel', 'OID', g.oid::text),
  (4, 'Genel', 'Yorum', obj_description(g.oid, 'pg_trigger')),
  (5, 'Tanım', 'Durum', CASE g.tgenabled WHEN 'O' THEN 'etkin' WHEN 'D' THEN 'devre dışı' WHEN 'R' THEN 'yalnızca replica' WHEN 'A' THEN 'her zaman' END),
  (6, 'Tanım', 'Fonksiyon', quote_ident(pn.nspname) || '.' || quote_ident(p.proname)),
  (7, 'Tanım', 'Tanım', pg_get_triggerdef(g.oid, true))
) AS v(n, section, label, value)
WHERE g.oid = $1
ORDER BY v.n
