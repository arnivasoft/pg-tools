-- Nesne yetkileri: ilişki (tablo/görünüm/MV/sequence). $1 = pg_class oid'i (yoksa satır yok → NotFound).
-- Etkin ACL (Plan 20 Karar 4): NULL relacl → acldefault ('s' sequence, 'r' diğerleri); boş ACL ('{}') → tek satır,
-- yetki sütunları NULL. Veren ayrımı (Karar 5): by_owner = veren sahip. Adlar ham metin (tırnaklama üreticide).
SELECT n.nspname::text AS schema,
       c.relname::text AS name,
       NULL::text AS signature,
       pg_get_userbyid(c.relowner)::text AS owner,
       current_user::text AS viewer,
       c.relacl IS NOT NULL AS explicit,
       c.relkind::text AS subkind,
       (n.nspname IN ('pg_catalog', 'information_schema') OR n.nspname LIKE 'pg\_toast%') AS is_system,
       CASE WHEN a.grantee = 0 THEN NULL ELSE pg_get_userbyid(a.grantee)::text END AS grantee,
       pg_get_userbyid(a.grantor)::text AS grantor,
       a.grantor = c.relowner AS by_owner,
       a.privilege_type::text AS privilege,
       a.is_grantable AS grantable
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN LATERAL aclexplode(coalesce(c.relacl, acldefault(CASE WHEN c.relkind = 'S' THEN 's' ELSE 'r' END::"char", c.relowner))) a ON true
WHERE c.oid = $1
