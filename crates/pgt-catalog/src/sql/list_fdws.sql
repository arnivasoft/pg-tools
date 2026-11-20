SELECT w.oid, w.fdwname::text AS name, h.proname::text AS detail, false AS leaf,
       EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_foreign_data_wrapper'::regclass AND d.objid = w.oid AND d.deptype = 'e') AS from_extension
FROM pg_foreign_data_wrapper w
LEFT JOIN pg_proc h ON h.oid = w.fdwhandler
ORDER BY w.fdwname::text COLLATE "C"
