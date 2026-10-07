-- Görünüm/MV kolonları, attnum sırasıyla. $1 = ilişki oid'i. data_type: format_type (arama yolunda görünmeyen tip
-- şema nitelikli). has_default/comment: modellenmeyen kolon ekleri (görünümde ALTER VIEW … ALTER COLUMN … SET DEFAULT).
SELECT a.attname::text AS name,
       format_type(a.atttypid, a.atttypmod)::text AS data_type,
       a.atthasdef AS has_default,
       col_description(a.attrelid, a.attnum) AS comment
FROM pg_attribute a
WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY a.attnum
