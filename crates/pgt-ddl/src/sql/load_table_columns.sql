-- Tablo kolonları, attnum sırasıyla. $1 = tablo oid'i. PG 12+: attgenerated ('s' STORED, PG 18 'v' VIRTUAL).
-- data_type: format_type (arama yolunda görünmeyen tip şema nitelikli). expr: varsayılan ya da üretilmiş ifade
-- (pg_get_expr, pretty); kimlik kolonunda pg_attrdef yoktur. collation yalnızca tipin varsayılanından FARKLIysa.
-- identity_options: kimlik dizisi varsayılan dışı seçenek taşıyor mu (START/INCREMENT/MIN/MAX/CACHE/CYCLE; modellenmez).
SELECT a.attnum::int2 AS attnum,
       a.attname::text AS name,
       format_type(a.atttypid, a.atttypmod)::text AS data_type,
       a.attnotnull AS not_null,
       a.attidentity::text AS identity,
       a.attgenerated::text AS generated,
       pg_get_expr(d.adbin, d.adrelid, true) AS expr,
       CASE WHEN a.attcollation <> t.typcollation THEN co.collname::text END AS collation,
       col_description(a.attrelid, a.attnum) AS comment,
       EXISTS (SELECT 1 FROM pg_depend dp
               JOIN pg_sequence s ON s.seqrelid = dp.objid
               WHERE dp.classid = 'pg_class'::regclass AND dp.refclassid = 'pg_class'::regclass
                 AND dp.refobjid = a.attrelid AND dp.refobjsubid = a.attnum AND dp.deptype = 'i'
                 AND (s.seqincrement <> 1 OR s.seqstart <> 1 OR s.seqmin <> 1 OR s.seqcache <> 1 OR s.seqcycle
                      OR s.seqmax <> CASE s.seqtypid WHEN 'int2'::regtype THEN 32767
                                                     WHEN 'int4'::regtype THEN 2147483647
                                                     ELSE 9223372036854775807 END)) AS identity_options
FROM pg_attribute a
JOIN pg_type t ON t.oid = a.atttypid
LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
LEFT JOIN pg_collation co ON co.oid = a.attcollation
WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY a.attnum
