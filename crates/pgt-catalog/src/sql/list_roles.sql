SELECT r.oid, r.rolname::text AS name,
       CASE WHEN r.rolsuper THEN 'superuser' WHEN r.rolcanlogin THEN 'login' ELSE 'group' END AS detail,
       true AS leaf, (r.rolname LIKE 'pg\_%') AS system
FROM pg_roles r
ORDER BY (r.rolname LIKE 'pg\_%'), r.rolname COLLATE "C"
