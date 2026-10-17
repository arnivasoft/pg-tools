-- Trigger fonksiyonları: dönüşü `trigger` ve argümansız (PG trigger fonksiyonu argüman bildiremez). $1 = şema (NULL ise
-- kullanıcı şemaları + pg_catalog; diğer pg_* ve information_schema hariç). value = ad, detail = şema; en çok 5000.
SELECT p.proname::text AS value, n.nspname::text AS detail
FROM pg_proc p
JOIN pg_namespace n ON n.oid = p.pronamespace
WHERE p.prorettype = 'trigger'::regtype
  AND p.pronargs = 0
  AND CASE WHEN $1::text IS NULL THEN (left(n.nspname, 3) <> 'pg_' OR n.nspname = 'pg_catalog') AND n.nspname <> 'information_schema'
           ELSE n.nspname = $1::text END
ORDER BY (n.nspname = 'pg_catalog'), n.nspname, p.proname
LIMIT 5000
