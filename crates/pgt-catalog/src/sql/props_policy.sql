SELECT v.section, v.label, v.value
FROM pg_policy p
JOIN pg_class c ON c.oid = p.polrelid
JOIN pg_namespace n ON n.oid = c.relnamespace,
LATERAL (VALUES
  (1, 'Genel', 'Ad', p.polname::text),
  (2, 'Genel', 'Tablo', quote_ident(n.nspname) || '.' || quote_ident(c.relname)),
  (3, 'Genel', 'OID', p.oid::text),
  (4, 'Genel', 'Yorum', obj_description(p.oid, 'pg_policy')),
  (5, 'Tanım', 'Komut', CASE p.polcmd WHEN 'r' THEN 'SELECT' WHEN 'a' THEN 'INSERT' WHEN 'w' THEN 'UPDATE' WHEN 'd' THEN 'DELETE' ELSE 'ALL' END),
  (6, 'Tanım', 'Tür', CASE WHEN p.polpermissive THEN 'permissive' ELSE 'restrictive' END),
  (7, 'Tanım', 'Roller', CASE WHEN p.polroles = '{0}' THEN 'PUBLIC' ELSE (SELECT string_agg(quote_ident(r.rolname), ', ' ORDER BY r.rolname) FROM pg_roles r WHERE r.oid = ANY (p.polroles)) END),
  (8, 'Tanım', 'USING', pg_get_expr(p.polqual, p.polrelid, true)),
  (9, 'Tanım', 'WITH CHECK', pg_get_expr(p.polwithcheck, p.polrelid, true))
) AS v(n, section, label, value)
WHERE p.oid = $1
ORDER BY v.n
