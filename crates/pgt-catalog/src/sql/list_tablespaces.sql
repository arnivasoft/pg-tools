SELECT t.oid, t.spcname::text AS name, NULL::text AS detail, true AS leaf,
       (t.spcname LIKE 'pg\_%') AS system
FROM pg_tablespace t
ORDER BY (t.spcname LIKE 'pg\_%'), t.spcname COLLATE "C"
