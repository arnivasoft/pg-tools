-- Tablo kısıtları, ad sırasıyla (C collation; eşit adda oid). $1 = tablo oid'i. PG 15–17: NULLS NOT DISTINCT ve ON DELETE SET NULL (kolonlar).
-- columns/ref_columns: conkey/confkey sırasıyla kolon adları. include: PK/UNIQUE indeksinde anahtar sayısından sonraki
-- kolonlar. check_expr: pg_get_expr(conbin, conrelid, pretty). extra: modellenmeyen özellikler (dolu ise kısıt
-- "desteklenmeyen" listesine gider ve alter ona dokunmaz); PK/UNIQUE indeksinin varsayılan dışı tablespace'i ve
-- depolama seçenekleri de buradadır (DROP+ADD bunları kaybederdi).
SELECT con.oid AS oid,
       con.conname::text AS name,
       con.contype::text AS contype,
       ARRAY(SELECT a.attname::text FROM unnest(con.conkey) WITH ORDINALITY AS k(n, ord)
             JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.n ORDER BY k.ord)::text[] AS columns,
       ARRAY(SELECT a.attname::text FROM pg_index i CROSS JOIN LATERAL unnest(i.indkey::int2[]) WITH ORDINALITY AS k(n, ord)
             JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.n
             WHERE i.indexrelid = con.conindid AND con.contype IN ('p', 'u') AND k.ord > i.indnkeyatts ORDER BY k.ord)::text[] AS include,
       (con.contype = 'u' AND COALESCE((SELECT i.indnullsnotdistinct FROM pg_index i WHERE i.indexrelid = con.conindid), false)) AS nulls_not_distinct,
       CASE WHEN con.contype = 'c' THEN pg_get_expr(con.conbin, con.conrelid, true) END AS check_expr,
       con.connoinherit AS no_inherit,
       rn.nspname::text AS ref_schema,
       rc.relname::text AS ref_table,
       ARRAY(SELECT a.attname::text FROM unnest(con.confkey) WITH ORDINALITY AS k(n, ord)
             JOIN pg_attribute a ON a.attrelid = con.confrelid AND a.attnum = k.n ORDER BY k.ord)::text[] AS ref_columns,
       con.confmatchtype::text AS match_type,
       con.confupdtype::text AS on_update,
       con.confdeltype::text AS on_delete,
       con.condeferrable AS deferrable,
       con.condeferred AS initially_deferred,
       con.convalidated AS validated,
       NULLIF(concat_ws(', ',
           (SELECT NULLIF(concat_ws(', ',
                   CASE WHEN ic.reltablespace <> 0 THEN 'USING INDEX TABLESPACE ' || quote_ident(its.spcname) END,
                   CASE WHEN ic.reloptions IS NOT NULL THEN 'WITH (' || array_to_string(ic.reloptions, ', ') || ')' END), '')
            FROM pg_class ic LEFT JOIN pg_tablespace its ON its.oid = ic.reltablespace
            WHERE con.contype IN ('p', 'u') AND ic.oid = con.conindid),
           CASE WHEN con.confdelsetcols IS NOT NULL THEN 'ON DELETE SET NULL/DEFAULT (kolon listesi)' END), '') AS extra
FROM pg_constraint con
LEFT JOIN pg_class rc ON rc.oid = con.confrelid
LEFT JOIN pg_namespace rn ON rn.oid = rc.relnamespace
WHERE con.conrelid = $1
  -- Bölümlenmiş tabloya başvuran FK, hedefin her bölümü için aynı conrelid'li alt kısıt satırı üretir (PG 12+): ebeveyni
  -- aynı tabloda olan satırlar gizlenir. Bölümün ebeveynden gelen kısıtları (ebeveyn başka tabloda) görünür kalır.
  AND NOT EXISTS (SELECT 1 FROM pg_constraint pc WHERE pc.oid = con.conparentid AND pc.conrelid = con.conrelid)
ORDER BY con.conname::text COLLATE "C", con.oid
