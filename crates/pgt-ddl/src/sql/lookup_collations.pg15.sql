-- Kolon collation önerileri: veritabanı kodlamasıyla kullanılabilen ve arama yolunda görünen (adla yazılabilen)
-- collation'lar, ad sırasıyla. detail: sağlayıcı + locale. PG 15–16: ICU locale = colliculocale.
SELECT DISTINCT ON (c.collname) c.collname::text AS value,
       concat_ws(' · ', CASE c.collprovider WHEN 'c' THEN 'libc' WHEN 'i' THEN 'icu' WHEN 'd' THEN 'default'
                                            WHEN 'b' THEN 'builtin' END,
                 NULLIF(COALESCE(c.colliculocale, c.collcollate)::text, '')) AS detail
FROM pg_collation c
WHERE c.collencoding IN (-1, (SELECT d.encoding FROM pg_database d WHERE d.datname = current_database()))
  AND pg_collation_is_visible(c.oid)
ORDER BY c.collname, c.collencoding DESC
