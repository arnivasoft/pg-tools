-- Veri düzenleyici kolonları, attnum sırasıyla. $1 = ilişki oid'i. PG 12+: attgenerated ('s' STORED, PG 18 'v' VIRTUAL).
-- col_insert/col_update: ham kolon yetkisi (tablo düzeyi yetki de sayılır); insertable/updatable Rust'ta türetilir.
SELECT a.attnum::int2 AS attnum,
       a.attname::text AS name,
       a.atttypid AS type_oid,
       format_type(a.atttypid, a.atttypmod)::text AS type_display,
       a.attnotnull AS not_null,
       a.atthasdef AS has_default,
       a.attidentity::text AS identity,
       (a.attgenerated <> '') AS generated,
       has_column_privilege(a.attrelid, a.attnum, 'INSERT') AS col_insert,
       has_column_privilege(a.attrelid, a.attnum, 'UPDATE') AS col_update
FROM pg_attribute a
WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY a.attnum
