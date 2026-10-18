-- Bakım hedefi: tablo/bölümlenmiş tablo/MV. $1 = pg_class oid'i (yoksa satır yok → NotFound).
-- `indexes`: CLUSTER USING'de kullanılabilir indeksler (geçerli, kısmi olmayan, erişim yöntemi kümelemeyi
-- destekleyen), ad (bayt) sırasıyla; `cluster_index`: geçerli (`indisvalid`) `indisclustered` indeks (geçersiz indeksle
-- düz CLUSTER sunucuda başarısız olur). Adlar ham metin (tırnaklama üreticide).
SELECT n.nspname::text AS schema,
       c.relname::text AS name,
       c.relkind::text AS relkind,
       NULL::text AS table_name,
       (n.nspname IN ('pg_catalog', 'information_schema') OR n.nspname LIKE 'pg\_toast%') AS is_system,
       (n.nspname LIKE 'pg\_temp\_%' OR n.nspname LIKE 'pg\_toast\_temp\_%') AS is_temp,
       ARRAY(SELECT ic.relname::text
             FROM pg_index x
             JOIN pg_class ic ON ic.oid = x.indexrelid
             WHERE x.indrelid = c.oid
               AND x.indisvalid
               AND x.indpred IS NULL
               AND coalesce(pg_index_has_property(x.indexrelid, 'clusterable'), false)
             ORDER BY ic.relname) AS indexes,
       (SELECT ic.relname::text
        FROM pg_index x
        JOIN pg_class ic ON ic.oid = x.indexrelid
        WHERE x.indrelid = c.oid AND x.indisclustered AND x.indisvalid
        ORDER BY ic.relname
        LIMIT 1) AS cluster_index
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.oid = $1
