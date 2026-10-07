-- Veritabanı yerel ayar adları (PG 15–16): libc → collcollate + C ve POSIX; icu → colliculocale; builtin yok (boş).
-- $1 = sağlayıcı ('libc' | 'icu' | 'builtin'). detail: kodlamalar (yalnızca kodlamaya bağlı libc yerel ayarlarında).
WITH l AS (
    SELECT c.collcollate::text AS value, c.collencoding
    FROM pg_collation c
    WHERE $1::text = 'libc' AND c.collprovider = 'c'
    UNION ALL SELECT 'C', -1 WHERE $1::text = 'libc'
    UNION ALL SELECT 'POSIX', -1 WHERE $1::text = 'libc'
    UNION ALL
    SELECT c.colliculocale::text, c.collencoding
    FROM pg_collation c
    WHERE $1::text = 'icu' AND c.collprovider = 'i' AND c.colliculocale IS NOT NULL
)
SELECT value,
       NULLIF(string_agg(DISTINCT pg_encoding_to_char(collencoding), ', ') FILTER (WHERE collencoding <> -1), '') AS detail
FROM l
WHERE value <> ''
GROUP BY value
ORDER BY value COLLATE "C"
