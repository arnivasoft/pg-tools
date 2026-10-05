SELECT v.section, v.label, v.value
FROM pg_roles r,
LATERAL (VALUES
  (1, 'Genel', 'Ad', r.rolname::text),
  (2, 'Genel', 'OID', r.oid::text),
  (3, 'Genel', 'Yorum', shobj_description(r.oid, 'pg_authid')),
  (4, 'Yetkiler', 'Oturum açabilir', CASE WHEN r.rolcanlogin THEN 'evet' ELSE 'hayır' END),
  (5, 'Yetkiler', 'Superuser', CASE WHEN r.rolsuper THEN 'evet' ELSE 'hayır' END),
  (6, 'Yetkiler', 'Veritabanı oluşturabilir', CASE WHEN r.rolcreatedb THEN 'evet' ELSE 'hayır' END),
  (7, 'Yetkiler', 'Rol oluşturabilir', CASE WHEN r.rolcreaterole THEN 'evet' ELSE 'hayır' END),
  (8, 'Yetkiler', 'Kalıtım', CASE WHEN r.rolinherit THEN 'evet' ELSE 'hayır' END),
  (9, 'Yetkiler', 'Replikasyon', CASE WHEN r.rolreplication THEN 'evet' ELSE 'hayır' END),
  (10, 'Yetkiler', 'RLS atlayabilir', CASE WHEN r.rolbypassrls THEN 'evet' ELSE 'hayır' END),
  (11, 'Tanım', 'Bağlantı sınırı', CASE WHEN r.rolconnlimit >= 0 THEN r.rolconnlimit::text END),
  (12, 'Tanım', 'Geçerlilik sonu', r.rolvaliduntil::text),
  (13, 'Tanım', 'Üye olduğu roller', (SELECT string_agg(quote_ident(g.rolname), ', ' ORDER BY g.rolname) FROM pg_auth_members m JOIN pg_roles g ON g.oid = m.roleid WHERE m.member = r.oid))
) AS v(n, section, label, value)
WHERE r.oid = $1
ORDER BY v.n
