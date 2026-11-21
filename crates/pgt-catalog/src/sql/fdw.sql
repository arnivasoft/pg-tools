SELECT w.oid, w.fdwname::text AS name, quote_ident(w.fdwname) AS qname,
       pg_get_userbyid(w.fdwowner)::text AS owner, quote_ident(pg_get_userbyid(w.fdwowner)) AS qowner,
       obj_description(w.oid, 'pg_foreign_data_wrapper') AS comment,
       CASE WHEN w.fdwhandler <> 0 THEN w.fdwhandler::regproc::text END AS handler,
       CASE WHEN w.fdwvalidator <> 0 THEN w.fdwvalidator::regproc::text END AS validator,
       COALESCE(w.fdwoptions, '{}') AS options,
       ARRAY(SELECT quote_ident(o.k) FROM pg_options_to_table(w.fdwoptions) WITH ORDINALITY o(k, v, n) ORDER BY o.n) AS option_keys,
       (SELECT e.extname::text FROM pg_depend d JOIN pg_extension e ON e.oid = d.refobjid
        WHERE d.classid = 'pg_foreign_data_wrapper'::regclass AND d.objid = w.oid
          AND d.refclassid = 'pg_extension'::regclass AND d.deptype = 'e') AS extension
FROM pg_foreign_data_wrapper w
WHERE w.oid = $1
