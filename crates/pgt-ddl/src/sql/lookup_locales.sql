-- Veritabanı yerel ayar adları (PG 17+): libc → collcollate + C ve POSIX; icu ve builtin → colllocale.
-- $1 = sağlayıcı ('libc' | 'icu' | 'builtin'). detail: kodlamalar (yalnızca kodlamaya bağlı libc yerel ayarlarında).
WITH l AS (
    SELECT c.collcollate::text AS value, c.collencoding
    FROM pg_collation c
    WHERE $1::text = 'libc' AND c.collprovider = 'c'
    UNION ALL SELECT 'C', -1 WHERE $1::text = 'libc'
    UNION ALL SELECT 'POSIX', -1 WHERE $1::text = 'libc'
    UNION ALL
    SELECT c.colllocale::text, c.collencoding
    FROM pg_collation c
    WHERE c.colllocale IS NOT NULL
      AND (($1::text = 'icu' AND c.collprovider = 'i') OR ($1::text = 'builtin' AND c.collprovider = 'b'))
)
SELECT value,
       NULLIF(string_agg(DISTINCT pg_encoding_to_char(collencoding), ', ') FILTER (WHERE collencoding <> -1), '') AS detail
FROM l
WHERE value <> ''
GROUP BY value
ORDER BY value COLLATE "C"
