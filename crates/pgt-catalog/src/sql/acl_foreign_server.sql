-- acl_foreign_server.sql
SELECT grantee, privilege, grantable FROM (
  SELECT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE quote_ident(pg_get_userbyid(a.grantee)) END AS grantee,
         a.privilege_type AS privilege, a.is_grantable AS grantable,
         a.grantee = o.srvowner AS is_owner
  FROM pg_foreign_server o, LATERAL aclexplode(o.srvacl) a
  WHERE o.oid = $1
) s
ORDER BY is_owner DESC, grantee::text COLLATE "C", privilege
