-- Tablo kolonları sırayla; ayrıntı = tür. $1 = şema, $2 = tablo.
SELECT a.attname::text AS value, format_type(a.atttypid, a.atttypmod)::text AS detail
FROM pg_attribute a
JOIN pg_class c ON c.oid = a.attrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = $1::text AND c.relname = $2::text AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY a.attnum
