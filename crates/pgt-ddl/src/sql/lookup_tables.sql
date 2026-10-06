-- Tablolar ve bölümlenmiş tablolar. $1 = şema (NULL ise sistem şemaları hariç tümü); en çok 5000.
SELECT c.relname::text AS value, n.nspname::text AS detail
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.relkind IN ('r', 'p')
  AND CASE WHEN $1::text IS NULL THEN left(n.nspname, 3) <> 'pg_' AND n.nspname <> 'information_schema'
           ELSE n.nspname = $1::text END
ORDER BY n.nspname, c.relname
LIMIT 5000
