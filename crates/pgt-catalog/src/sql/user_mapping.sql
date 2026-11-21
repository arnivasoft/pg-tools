-- pg_user_mapping herkese açık değildir; pg_user_mappings görünümü umoptions'ı yetkisiz kullanıcıya NULL verir.
-- options_readable görünümün koşuludur (seçeneksiz eşleme ile okunamayan eşlemeyi ayırır).
SELECT u.umid AS oid,
       CASE WHEN u.umuser = 0 THEN 'PUBLIC' ELSE u.usename::text END AS role,
       CASE WHEN u.umuser = 0 THEN 'PUBLIC' ELSE quote_ident(u.usename) END AS qrole,
       s.srvname::text AS server, quote_ident(s.srvname) AS qserver,
       u.umoptions IS NOT NULL
         OR (u.umuser <> 0 AND u.usename = current_user AND (pg_has_role(s.srvowner, 'USAGE') OR has_server_privilege(s.oid, 'USAGE')))
         OR (u.umuser = 0 AND pg_has_role(s.srvowner, 'USAGE'))
         OR COALESCE((SELECT r.rolsuper FROM pg_roles r WHERE r.rolname = current_user), false) AS options_readable,
       COALESCE(u.umoptions, '{}') AS options,
       ARRAY(SELECT quote_ident(o.k) FROM pg_options_to_table(u.umoptions) WITH ORDINALITY o(k, v, n) ORDER BY o.n) AS option_keys
FROM pg_user_mappings u
JOIN pg_foreign_server s ON s.oid = u.srvid
WHERE u.umid = $1
