SELECT a.attnum, a.attname::text AS name, format_type(a.atttypid, a.atttypmod) AS type_name, a.attnotnull AS not_null,
       EXISTS (
           SELECT 1 FROM pg_constraint k
           WHERE k.conrelid = a.attrelid AND k.contype = 'p' AND a.attnum = ANY (k.conkey)
       ) AS is_pk
FROM pg_attribute a
WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY a.attnum
