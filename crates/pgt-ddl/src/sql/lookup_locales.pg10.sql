-- Veritabanı yerel ayar adları (PG 10–14: yalnızca libc; ICU/builtin veritabanı sağlayıcısı yok → boş).
-- $1 = sağlayıcı ('libc' | 'icu' | 'builtin'). libc: collcollate değerleri + C ve POSIX. detail: kodlamalar.
WITH l AS (
    SELECT c.collcollate::text AS value, c.collencoding
    FROM pg_collation c
    WHERE $1::text = 'libc' AND c.collprovider = 'c'
    UNION ALL SELECT 'C', -1 WHERE $1::text = 'libc'
    UNION ALL SELECT 'POSIX', -1 WHERE $1::text = 'libc'
)
SELECT value,
       NULLIF(string_agg(DISTINCT pg_encoding_to_char(collencoding), ', ') FILTER (WHERE collencoding <> -1), '') AS detail
FROM l
WHERE value <> ''
GROUP BY value
ORDER BY value COLLATE "C"
