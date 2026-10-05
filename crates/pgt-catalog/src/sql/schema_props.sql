SELECT n.nspname::text AS name, n.oid, pg_get_userbyid(n.nspowner)::text AS owner,
       obj_description(n.oid, 'pg_namespace') AS comment,
       array_to_string(n.nspacl, ', ') AS acl
FROM pg_namespace n
WHERE n.oid = $1
