-- Rolün üye olduğu roller (PG 16+: üyelik başına INHERIT/SET seçenekleri ve grantor). $1 = üye rol oid'i.
-- Aynı (rol, üye) için birden çok grantor satırı olabilir → grants > 1 (Rust'ta "desteklenmeyen"). Tek satırda
-- grantor_is_bootstrap = grantor önyükleme süper kullanıcısı (oid 10) mı: değilse üyelik salt okunur (GRANTED BY'sız
-- REVOKE PG 16+'da WARNING ile etkisizdir). Seçenekler ve grantor yalnızca tek satırda anlamlı.
SELECT g.rolname::text AS role,
       bool_or(m.admin_option) AS admin,
       count(*)::int4 AS grants,
       bool_and(m.inherit_option) AS inherit_option,
       bool_and(m.set_option) AS set_option,
       bool_and(m.grantor = 10) AS grantor_is_bootstrap,
       min(COALESCE(gr.rolname::text, m.grantor::text)) AS grantor
FROM pg_auth_members m
JOIN pg_roles g ON g.oid = m.roleid
LEFT JOIN pg_roles gr ON gr.oid = m.grantor
WHERE m.member = $1
GROUP BY g.rolname
ORDER BY g.rolname::text COLLATE "C"
