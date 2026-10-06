-- Tablo kısıtları, ad sırasıyla (C collation; eşit adda oid). $1 = tablo oid'i. PG 10: INCLUDE (indnkeyatts) ve conparentid yok.
-- columns/ref_columns: conkey/confkey sırasıyla kolon adları. include: PK/UNIQUE indeksinde anahtar sayısından sonraki
-- kolonlar. check_expr: pg_get_expr(conbin, conrelid, pretty). extra: modellenmeyen özellikler (dolu ise kısıt
-- "desteklenmeyen" listesine gider ve alter ona dokunmaz).
SELECT con.oid AS oid,
       con.conname::text AS name,
       con.contype::text AS contype,
       ARRAY(SELECT a.attname::text FROM unnest(con.conkey) WITH ORDINALITY AS k(n, ord)
             JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.n ORDER BY k.ord)::text[] AS columns,
       ARRAY[]::text[] AS include,
       false AS nulls_not_distinct,
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
       NULL::text AS extra
FROM pg_constraint con
LEFT JOIN pg_class rc ON rc.oid = con.confrelid
LEFT JOIN pg_namespace rn ON rn.oid = rc.relnamespace
WHERE con.conrelid = $1
ORDER BY con.conname::text COLLATE "C", con.oid
