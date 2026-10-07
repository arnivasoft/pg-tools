-- Satır kimliği anahtarı adayları. $1 = ilişki oid'i. PG 10: indnkeyatts yok (INCLUDE PG 11+) → tüm kolonlar anahtar (indnatts).
-- (a) PK kısıtı: ad = kısıt adı, kolonlar conkey sırasıyla; geçerlilik/hazırlık kısıtın indeksinden (conindid).
-- (b) PK'nın kendi indeksi dışındaki benzersiz indeksler: ad = indeks adı, kolonlar indkey sırasıyla (tüm
--     indnatts öğe; int2vector 0 tabanlı); all_not_null = düz anahtar kolonlarının hepsi attnotnull.
-- has_expr: anahtar öğelerinden biri ≤ 0 — 0 ifade; < 0 sistem kolonu (ör. PG < 12 WITH OIDS `oid`; kaynak
--     kolonlarında yer almaz, anahtar olamaz). Seçim kuralı Rust'ta (choose_key).
SELECT true AS is_primary,
       con.conname::text AS name,
       ARRAY(SELECT a.attname::text
             FROM unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord)
             JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.attnum
             ORDER BY k.ord)::text[] AS columns,
       COALESCE(i.indisvalid, false) AS valid,
       COALESCE(i.indisready, false) AS ready,
       false AS partial,
       EXISTS (SELECT 1 FROM unnest(con.conkey) AS k(attnum) WHERE k.attnum <= 0) AS has_expr,
       true AS all_not_null
FROM pg_constraint con
LEFT JOIN pg_index i ON i.indexrelid = con.conindid
WHERE con.conrelid = $1 AND con.contype = 'p'
UNION ALL
SELECT false AS is_primary,
       c.relname::text AS name,
       ARRAY(SELECT a.attname::text
             FROM generate_series(0, i.indnatts::int4 - 1) AS k(n)
             JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = i.indkey[k.n]
             ORDER BY k.n)::text[] AS columns,
       i.indisvalid AS valid,
       i.indisready AS ready,
       (i.indpred IS NOT NULL) AS partial,
       EXISTS (SELECT 1 FROM generate_series(0, i.indnatts::int4 - 1) AS k(n) WHERE i.indkey[k.n] <= 0) AS has_expr,
       NOT EXISTS (SELECT 1
                   FROM generate_series(0, i.indnatts::int4 - 1) AS k(n)
                   JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = i.indkey[k.n]
                   WHERE NOT a.attnotnull) AS all_not_null
FROM pg_index i
JOIN pg_class c ON c.oid = i.indexrelid
WHERE i.indrelid = $1 AND i.indisunique AND NOT i.indisprimary
