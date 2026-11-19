-- Nesne yetkileri: tablespace (bakım bağlantısında okunur). $1 = pg_tablespace oid'i (yoksa satır yok → NotFound).
-- Etkin ACL: NULL spcacl → acldefault('t') (sahip CREATE). is_system yalnız pg_global (GRANT anlamsız → salt okunur).
SELECT NULL::text AS schema,
       t.spcname::text AS name,
       NULL::text AS signature,
       pg_get_userbyid(t.spcowner)::text AS owner,
       current_user::text AS viewer,
       t.spcacl IS NOT NULL AS explicit,
       't'::text AS subkind,
       (t.spcname = 'pg_global') AS is_system,
       CASE WHEN a.grantee = 0 THEN NULL ELSE pg_get_userbyid(a.grantee)::text END AS grantee,
       pg_get_userbyid(a.grantor)::text AS grantor,
       a.grantor = t.spcowner AS by_owner,
       a.privilege_type::text AS privilege,
       a.is_grantable AS grantable
FROM pg_tablespace t
LEFT JOIN LATERAL aclexplode(coalesce(t.spcacl, acldefault('t', t.spcowner))) a ON true
WHERE t.oid = $1
