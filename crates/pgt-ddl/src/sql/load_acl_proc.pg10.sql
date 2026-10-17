-- Nesne yetkileri: fonksiyon (PG 10; `prokind` yok → proisagg/proiswindow). $1 = pg_proc oid'i.
-- Etkin ACL: NULL proacl → acldefault('f') (sahip + PUBLIC EXECUTE). İmza `pg_get_function_identity_arguments`.
SELECT n.nspname::text AS schema,
       p.proname::text AS name,
       pg_get_function_identity_arguments(p.oid) AS signature,
       pg_get_userbyid(p.proowner)::text AS owner,
       current_user::text AS viewer,
       p.proacl IS NOT NULL AS explicit,
       CASE WHEN p.proisagg THEN 'a' WHEN p.proiswindow THEN 'w' ELSE 'f' END AS subkind,
       (n.nspname IN ('pg_catalog', 'information_schema') OR n.nspname LIKE 'pg\_toast%') AS is_system,
       CASE WHEN a.grantee = 0 THEN NULL ELSE pg_get_userbyid(a.grantee)::text END AS grantee,
       pg_get_userbyid(a.grantor)::text AS grantor,
       a.grantor = p.proowner AS by_owner,
       a.privilege_type::text AS privilege,
       a.is_grantable AS grantable
FROM pg_proc p
JOIN pg_namespace n ON n.oid = p.pronamespace
LEFT JOIN LATERAL aclexplode(coalesce(p.proacl, acldefault('f', p.proowner))) a ON true
WHERE p.oid = $1
