-- acl_type.sql
SELECT grantee, privilege, grantable FROM (
  SELECT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE quote_ident(pg_get_userbyid(a.grantee)) END AS grantee,
         a.privilege_type AS privilege, a.is_grantable AS grantable,
         a.grantee = o.typowner AS is_owner
  FROM pg_type o, LATERAL aclexplode(o.typacl) a
  WHERE o.oid = $1
) s
ORDER BY is_owner DESC, grantee::text COLLATE "C", privilege
