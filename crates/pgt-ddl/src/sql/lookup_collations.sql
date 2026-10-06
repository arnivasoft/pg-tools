-- Kolon collation önerileri: veritabanı kodlamasıyla kullanılabilen, pg_catalog ya da kullanıcı şemalarındaki (pg_* ve
-- information_schema hariç) collation'lar, ad sırasıyla. Üretici collation'ı TEK tanımlayıcı olarak (quote_ident)
-- yazdığından value niteliksiz addır; birden fazla şemada aynı adla bulunan (belirsiz) collation'lar listelenmez.
-- detail: sağlayıcı · locale, pg_catalog dışındakilerde şema da (uygulamada adın arama yolundan çözülmesi gerekir).
-- PG 17+: locale = colllocale (ICU/builtin) ya da collcollate.
WITH usable AS (
    SELECT c.collname::text AS name, n.nspname::text AS schema, c.collencoding,
           CASE c.collprovider WHEN 'c' THEN 'libc' WHEN 'i' THEN 'icu' WHEN 'd' THEN 'default'
                               WHEN 'b' THEN 'builtin' END AS provider,
           NULLIF(COALESCE(c.colllocale, c.collcollate)::text, '') AS locale
    FROM pg_collation c
    JOIN pg_namespace n ON n.oid = c.collnamespace
    WHERE c.collencoding IN (-1, (SELECT d.encoding FROM pg_database d WHERE d.datname = current_database()))
      AND (n.nspname = 'pg_catalog' OR (left(n.nspname, 3) <> 'pg_' AND n.nspname <> 'information_schema'))
),
unique_names AS (
    SELECT name FROM usable GROUP BY name HAVING count(DISTINCT schema) = 1
)
SELECT DISTINCT ON (u.name) u.name AS value,
       concat_ws(' · ', u.provider, u.locale, CASE WHEN u.schema <> 'pg_catalog' THEN 'şema ' || u.schema END) AS detail
FROM usable u
JOIN unique_names USING (name)
ORDER BY u.name, u.collencoding DESC
