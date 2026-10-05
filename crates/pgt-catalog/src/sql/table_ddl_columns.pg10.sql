SELECT quote_ident(a.attname) AS name, format_type(a.atttypid, a.atttypmod) AS type_name, a.attnotnull AS not_null,
       pg_get_expr(d.adbin, d.adrelid) AS default_expr,
       a.attidentity::text AS identity, ''::text AS generated,
       CASE WHEN a.attcollation <> 0 AND a.attcollation <> t.typcollation
            THEN quote_ident(cn.nspname) || '.' || quote_ident(co.collname) END AS collation,
       quote_literal(col_description(a.attrelid, a.attnum)) AS comment,
       CASE WHEN a.attidentity <> '' AND (s.seqstart <> 1 OR s.seqincrement <> 1 OR s.seqmin <> 1 OR s.seqcache <> 1 OR s.seqcycle
                OR s.seqmax <> CASE s.seqtypid WHEN 'int2'::regtype THEN 32767 WHEN 'int4'::regtype THEN 2147483647 ELSE 9223372036854775807 END)
            THEN format('START WITH %s INCREMENT BY %s MINVALUE %s MAXVALUE %s CACHE %s%s', s.seqstart, s.seqincrement, s.seqmin, s.seqmax, s.seqcache,
                        CASE WHEN s.seqcycle THEN ' CYCLE' ELSE '' END) END AS identity_opts
FROM pg_attribute a
JOIN pg_type t ON t.oid = a.atttypid
LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
LEFT JOIN pg_collation co ON co.oid = a.attcollation
LEFT JOIN pg_namespace cn ON cn.oid = co.collnamespace
LEFT JOIN pg_depend dep ON dep.classid = 'pg_class'::regclass AND dep.refobjid = a.attrelid AND dep.refobjsubid = a.attnum
                       AND dep.refclassid = 'pg_class'::regclass AND dep.deptype = 'i' AND a.attidentity <> ''
LEFT JOIN pg_sequence s ON s.seqrelid = dep.objid
WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
  AND ($2::int2 IS NULL OR a.attnum = $2::int2)
  AND (a.attislocal OR a.attinhcount = 0 OR $2::int2 IS NOT NULL)
ORDER BY a.attnum
