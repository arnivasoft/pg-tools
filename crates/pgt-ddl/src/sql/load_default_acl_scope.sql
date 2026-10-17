-- Varsayılan yetkiler kapsamı: rol ve (varsa) şema çözümü. $1 = rol adı (NULL → current_user), $2 = pg_namespace
-- oid'i (NULL → genel kapsam). Rol yoksa satır yok (→ NotFound "rol"); şema yoksa schema_found = false (→ NotFound
-- "şema"). Adlar ham metin (tırnaklama üreticide).
SELECT r.oid AS role_oid,
       r.rolname::text AS role,
       current_user::text AS viewer,
       n.oid IS NOT NULL AS schema_found,
       n.nspname::text AS schema,
       coalesce(n.nspname IN ('pg_catalog', 'information_schema') OR n.nspname LIKE 'pg\_toast%', false) AS is_system
FROM pg_roles r
LEFT JOIN pg_namespace n ON n.oid = $2::oid
WHERE r.rolname = coalesce($1::text, current_user::text)
