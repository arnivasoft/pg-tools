-- acl_relation.sql
SELECT grantee, privilege, grantable FROM (
  SELECT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE quote_ident(pg_get_userbyid(a.grantee)) END AS grantee,
         a.privilege_type AS privilege, a.is_grantable AS grantable
  FROM pg_class o, LATERAL aclexplode(o.relacl) a
  WHERE o.oid = $1
) s
ORDER BY grantee COLLATE "C", privilege
