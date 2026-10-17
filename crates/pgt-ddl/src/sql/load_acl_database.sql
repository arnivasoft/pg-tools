-- Nesne yetkileri: veritabanı (bakım bağlantısında okunur). $1 = pg_database oid'i (yoksa satır yok → NotFound).
-- Etkin ACL: NULL datacl → acldefault('d') (sahip CREATE, TEMPORARY, CONNECT + PUBLIC TEMPORARY, CONNECT).
SELECT NULL::text AS schema,
       d.datname::text AS name,
       NULL::text AS signature,
       pg_get_userbyid(d.datdba)::text AS owner,
       current_user::text AS viewer,
       d.datacl IS NOT NULL AS explicit,
       'd'::text AS subkind,
       false AS is_system,
       CASE WHEN a.grantee = 0 THEN NULL ELSE pg_get_userbyid(a.grantee)::text END AS grantee,
       pg_get_userbyid(a.grantor)::text AS grantor,
       a.grantor = d.datdba AS by_owner,
       a.privilege_type::text AS privilege,
       a.is_grantable AS grantable
FROM pg_database d
LEFT JOIN LATERAL aclexplode(coalesce(d.datacl, acldefault('d', d.datdba))) a ON true
WHERE d.oid = $1
