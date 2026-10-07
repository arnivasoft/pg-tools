-- Rolün üye olduğu roller (PG 16+: üyelik başına INHERIT/SET seçenekleri; aynı (rol, üye) için birden çok grantor
-- satırı olabilir → grants > 1, Rust'ta "desteklenmeyen"). $1 = üye rol oid'i. Seçenekler yalnızca tek satırda anlamlı.
SELECT g.rolname::text AS role,
       bool_or(m.admin_option) AS admin,
       count(*)::int4 AS grants,
       bool_and(m.inherit_option) AS inherit_option,
       bool_and(m.set_option) AS set_option
FROM pg_auth_members m
JOIN pg_roles g ON g.oid = m.roleid
WHERE m.member = $1
GROUP BY g.rolname
ORDER BY g.rolname::text COLLATE "C"
