SELECT v.section, v.label, v.value
FROM pg_proc p
JOIN pg_namespace n ON n.oid = p.pronamespace
JOIN pg_language l ON l.oid = p.prolang,
LATERAL (VALUES
  (1, 'Genel', 'Ad', p.proname::text),
  (2, 'Genel', 'Şema', n.nspname::text),
  (3, 'Genel', 'OID', p.oid::text),
  (4, 'Genel', 'Sahip', pg_get_userbyid(p.proowner)::text),
  (5, 'Genel', 'Yorum', obj_description(p.oid, 'pg_proc')),
  (6, 'Tanım', 'Argümanlar', regexp_replace(pg_get_function_arguments(p.oid), '(^|, )IN ', '\1', 'g')),
  (7, 'Tanım', 'Dönüş tipi', pg_get_function_result(p.oid)),
  (8, 'Tanım', 'Dil', l.lanname::text),
  (9, 'Tanım', 'Değişkenlik', CASE p.provolatile WHEN 'i' THEN 'immutable' WHEN 's' THEN 'stable' ELSE 'volatile' END),
  (10, 'Tanım', 'Güvenlik', CASE WHEN p.prosecdef THEN 'security definer' ELSE 'security invoker' END),
  (11, 'Tanım', 'Paralel', CASE p.proparallel WHEN 's' THEN 'safe' WHEN 'r' THEN 'restricted' ELSE 'unsafe' END)
) AS v(n, section, label, value)
WHERE p.oid = $1
ORDER BY v.n
