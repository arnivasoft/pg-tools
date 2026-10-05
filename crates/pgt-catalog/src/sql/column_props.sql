SELECT a.attname::text AS name, a.attnum, format_type(a.atttypid, a.atttypmod) AS type_name, a.attnotnull AS not_null,
       pg_get_expr(d.adbin, d.adrelid) AS default_expr,
       CASE a.attidentity WHEN 'a' THEN 'ALWAYS' WHEN 'd' THEN 'BY DEFAULT' END AS identity,
       CASE a.attgenerated WHEN 's' THEN 'STORED' WHEN 'v' THEN 'VIRTUAL' END AS generated,
       CASE WHEN a.attcollation <> 0 AND a.attcollation <> t.typcollation THEN quote_ident(co.collname) END AS collation,
       col_description(a.attrelid, a.attnum) AS comment
FROM pg_attribute a
JOIN pg_type t ON t.oid = a.atttypid
LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
LEFT JOIN pg_collation co ON co.oid = a.attcollation
WHERE a.attrelid = $1 AND a.attnum = $2 AND NOT a.attisdropped
