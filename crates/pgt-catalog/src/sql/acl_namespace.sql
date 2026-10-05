-- acl_namespace.sql
SELECT grantee, privilege, grantable FROM (
  SELECT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE quote_ident(pg_get_userbyid(a.grantee)) END AS grantee,
         a.privilege_type AS privilege, a.is_grantable AS grantable,
         a.grantee = o.nspowner AS is_owner
  FROM pg_namespace o, LATERAL aclexplode(o.nspacl) a
  WHERE o.oid = $1
) s
-- Sahip rolü (adı ne olursa olsun) önce, sonra diğerleri ada göre: sıra ortamdan bağımsız.
ORDER BY is_owner DESC, grantee::text COLLATE "C", privilege
