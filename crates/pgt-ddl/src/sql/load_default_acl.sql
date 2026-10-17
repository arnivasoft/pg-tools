-- Varsayılan yetki girdileri (Plan 20 Karar 12). $1 = rol oid'i, $2 = pg_namespace oid'i (NULL → genel kapsam).
-- scope 'g': genel kapsamın etkin değeri — desteklenen her tür için coalesce(defaclacl, acldefault(...)) (pg_default_acl
--   harfleri r/S/f/T/n; acldefault harfleri r/s/f/T/n — sequence 's', 'S' acldefault'ta yabancı sunucudur); genel
--   kapsamda yüklerken desteklenmeyen türlerin genel girdileri de (ör. PG 18 'L' large object).
-- scope 's': yalnız o şemanın girdileri ($2 doluysa).
-- Boş ACL ('{}') → tek satır, yetki sütunları NULL. by_role: veren = rol (PG her zaman rolü yazar).
WITH types(objtype, deflt) AS (
    VALUES ('r'::"char", 'r'::"char"), ('S', 's'), ('f', 'f'), ('T', 'T'), ('n', 'n')
), entries AS (
    SELECT 'g'::text AS scope, t.objtype, coalesce(d.defaclacl, acldefault(t.deflt, $1::oid)) AS acl
    FROM types t
    LEFT JOIN pg_default_acl d ON d.defaclrole = $1::oid AND d.defaclnamespace = 0 AND d.defaclobjtype = t.objtype
    UNION ALL
    SELECT 'g', d.defaclobjtype, d.defaclacl
    FROM pg_default_acl d
    WHERE $2::oid IS NULL AND d.defaclrole = $1::oid AND d.defaclnamespace = 0
      AND d.defaclobjtype NOT IN (SELECT objtype FROM types)
    UNION ALL
    SELECT 's', d.defaclobjtype, d.defaclacl
    FROM pg_default_acl d
    WHERE d.defaclrole = $1::oid AND d.defaclnamespace = $2::oid
)
SELECT r.scope,
       r.objtype::text AS objtype,
       CASE WHEN a.grantee = 0 THEN NULL ELSE pg_get_userbyid(a.grantee)::text END AS grantee,
       a.grantor = $1::oid AS by_role,
       a.privilege_type::text AS privilege,
       a.is_grantable AS grantable
FROM entries r
LEFT JOIN LATERAL aclexplode(r.acl) a ON true
