-- Nesne yetkileri: şema. $1 = pg_namespace oid'i (yoksa satır yok → NotFound).
-- Etkin ACL: NULL nspacl → acldefault('n') (yalnız sahip USAGE, CREATE).
SELECT NULL::text AS schema,
       n.nspname::text AS name,
       NULL::text AS signature,
       pg_get_userbyid(n.nspowner)::text AS owner,
       current_user::text AS viewer,
       n.nspacl IS NOT NULL AS explicit,
       'n'::text AS subkind,
       (n.nspname IN ('pg_catalog', 'information_schema') OR n.nspname LIKE 'pg\_toast%') AS is_system,
       CASE WHEN a.grantee = 0 THEN NULL ELSE pg_get_userbyid(a.grantee)::text END AS grantee,
       pg_get_userbyid(a.grantor)::text AS grantor,
       a.grantor = n.nspowner AS by_owner,
       a.privilege_type::text AS privilege,
       a.is_grantable AS grantable
FROM pg_namespace n
LEFT JOIN LATERAL aclexplode(coalesce(n.nspacl, acldefault('n', n.nspowner))) a ON true
WHERE n.oid = $1
