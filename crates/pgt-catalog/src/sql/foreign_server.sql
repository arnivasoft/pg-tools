SELECT s.oid, s.srvname::text AS name, quote_ident(s.srvname) AS qname,
       pg_get_userbyid(s.srvowner)::text AS owner, quote_ident(pg_get_userbyid(s.srvowner)) AS qowner,
       obj_description(s.oid, 'pg_foreign_server') AS comment,
       w.fdwname::text AS fdw, quote_ident(w.fdwname) AS qfdw,
       s.srvtype::text AS type, s.srvversion::text AS version,
       COALESCE(s.srvoptions, '{}') AS options,
       ARRAY(SELECT quote_ident(o.k) FROM pg_options_to_table(s.srvoptions) WITH ORDINALITY o(k, v, n) ORDER BY o.n) AS option_keys,
       (SELECT e.extname::text FROM pg_depend d JOIN pg_extension e ON e.oid = d.refobjid
        WHERE d.classid = 'pg_foreign_server'::regclass AND d.objid = s.oid
          AND d.refclassid = 'pg_extension'::regclass AND d.deptype = 'e') AS extension
FROM pg_foreign_server s
JOIN pg_foreign_data_wrapper w ON w.oid = s.srvfdw
WHERE s.oid = $1
