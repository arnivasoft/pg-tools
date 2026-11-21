SELECT u.umid AS oid, CASE WHEN u.umuser = 0 THEN 'PUBLIC' ELSE u.usename::text END AS name, NULL::text AS detail, true AS leaf
FROM pg_user_mappings u
WHERE u.srvid = $1
ORDER BY (u.umuser = 0) DESC, u.usename::text COLLATE "C"
