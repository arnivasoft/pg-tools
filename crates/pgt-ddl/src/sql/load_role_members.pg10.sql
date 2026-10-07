-- Rolün üye olduğu roller (PG 10–15: (rol, üye) başına tek satır; INHERIT/SET seçenekleri yok). $1 = üye rol oid'i.
SELECT g.rolname::text AS role,
       bool_or(m.admin_option) AS admin,
       count(*)::int4 AS grants,
       NULL::bool AS inherit_option,
       NULL::bool AS set_option
FROM pg_auth_members m
JOIN pg_roles g ON g.oid = m.roleid
WHERE m.member = $1
GROUP BY g.rolname
ORDER BY g.rolname::text COLLATE "C"
