-- Kolon yetkileri (salt okunur; Table/View/MV). $1 = pg_class oid'i. Yalnız açık attacl (NULL = kolon yetkisi yok).
SELECT a.attnum::int4 AS attnum,
       a.attname::text AS column_name,
       CASE WHEN x.grantee = 0 THEN NULL ELSE pg_get_userbyid(x.grantee)::text END AS grantee,
       pg_get_userbyid(x.grantor)::text AS grantor,
       x.privilege_type::text AS privilege,
       x.is_grantable AS grantable
FROM pg_attribute a
CROSS JOIN LATERAL aclexplode(a.attacl) x
WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped AND a.attacl IS NOT NULL
ORDER BY a.attnum
