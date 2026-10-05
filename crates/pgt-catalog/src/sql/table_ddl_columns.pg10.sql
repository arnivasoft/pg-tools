SELECT quote_ident(a.attname) AS name, format_type(a.atttypid, a.atttypmod) AS type_name, a.attnotnull AS not_null,
       pg_get_expr(d.adbin, d.adrelid) AS default_expr,
       a.attidentity::text AS identity, ''::text AS generated,
       CASE WHEN a.attcollation <> 0 AND a.attcollation <> t.typcollation
            THEN quote_ident(cn.nspname) || '.' || quote_ident(co.collname) END AS collation,
       quote_literal(col_description(a.attrelid, a.attnum)) AS comment
FROM pg_attribute a
JOIN pg_type t ON t.oid = a.atttypid
LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
LEFT JOIN pg_collation co ON co.oid = a.attcollation
LEFT JOIN pg_namespace cn ON cn.oid = co.collnamespace
WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
  AND ($2::int2 IS NULL OR a.attnum = $2::int2)
ORDER BY a.attnum
