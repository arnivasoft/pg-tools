-- Nesne yetkileri: tip/domain. $1 = pg_type oid'i (yoksa satır yok → NotFound).
-- Etkin ACL: NULL typacl → acldefault('T') (sahip + PUBLIC USAGE). subkind = typtype ('d' ⇔ Domain).
-- Salt okunurluk girdileri (Karar 11): is_array (PG'nin dizi tipi kuralı: typelem <> 0 ve typlen = -1, ya da kategori
-- 'A'), multirange (typtype 'm'), row_relkind (composite'in ilişkisi; 'c' dışı = tablo/görünüm satır tipi).
SELECT n.nspname::text AS schema,
       t.typname::text AS name,
       NULL::text AS signature,
       pg_get_userbyid(t.typowner)::text AS owner,
       current_user::text AS viewer,
       t.typacl IS NOT NULL AS explicit,
       t.typtype::text AS subkind,
       (n.nspname IN ('pg_catalog', 'information_schema') OR n.nspname LIKE 'pg\_toast%') AS is_system,
       ((t.typelem <> 0 AND t.typlen = -1) OR t.typcategory = 'A') AS is_array,
       (SELECT r.relkind::text FROM pg_class r WHERE r.oid = t.typrelid) AS row_relkind,
       CASE WHEN a.grantee = 0 THEN NULL ELSE pg_get_userbyid(a.grantee)::text END AS grantee,
       pg_get_userbyid(a.grantor)::text AS grantor,
       a.grantor = t.typowner AS by_owner,
       a.privilege_type::text AS privilege,
       a.is_grantable AS grantable
FROM pg_type t
JOIN pg_namespace n ON n.oid = t.typnamespace
LEFT JOIN LATERAL aclexplode(coalesce(t.typacl, acldefault('T', t.typowner))) a ON true
WHERE t.oid = $1
