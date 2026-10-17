-- Kolon tipi önerileri. $1 = arama metni (NULL = tümü; büyük/küçük harf duyarsız alt dize, LIKE kalıbı DEĞİL).
-- value: format_type(oid, NULL) — validate_type'tan geçen kullanılabilir tip ifadesi; arama yolunda görünmeyen tip
-- şema niteliklidir, dizi tipi `x[]` biçimindedir (`_x` değil). detail: tür (base/enum/domain/composite/range/
-- multirange/array). Dışarıda: sözde tipler (internal, record, any*…), tanımsız (kabuk) tipler, tablo/görünüm satır
-- tipleri (yalnızca CREATE TYPE AS bileşik tipleri), pg_toast/information_schema/geçici şemalar, katalog iç tipleri.
-- Sıra: önek eşleşmesi önce, sonra pg_catalog, dizi olmayan, kısa ad; en çok 200.
-- $2 = `pgt_core::pgtext::VALID_UTF8_ESCAPED_RE`: adı geçerli UTF-8 olmayan multirange (PG'nin kestiği otomatik ad) ve
-- dizisi dışarıda — `lower()` öyle bir adda sorgunun tamamını 22021 ile düşürür; ad SQL'e de yazılamaz.
WITH usable AS (
    SELECT t.oid, t.typname::text AS typname, t.typarray, n.nspname::text AS nspname,
           CASE t.typtype WHEN 'b' THEN 'base' WHEN 'e' THEN 'enum' WHEN 'd' THEN 'domain' WHEN 'c' THEN 'composite'
                          WHEN 'r' THEN 'range' ELSE 'multirange' END AS kind
    FROM pg_type t
    JOIN pg_namespace n ON n.oid = t.typnamespace
    WHERE t.typisdefined
      AND t.typtype IN ('b', 'e', 'd', 'c', 'r', 'm')
      AND (t.typtype <> 'm' OR encode(textsend(t.typname::text), 'escape') ~ $2::text)
      AND NOT (t.typcategory = 'A' AND t.typelem <> 0)
      AND n.nspname <> 'information_schema'
      AND (left(n.nspname, 3) <> 'pg_' OR n.nspname = 'pg_catalog')
      AND (t.typtype <> 'c' OR EXISTS (SELECT 1 FROM pg_class c WHERE c.oid = t.typrelid AND c.relkind = 'c'))
      AND NOT (n.nspname = 'pg_catalog' AND t.typname IN ('pg_node_tree', 'pg_ndistinct', 'pg_dependencies',
               'pg_mcv_list', 'pg_ddl_command', 'pg_brin_bloom_summary', 'pg_brin_minmax_multi_summary', 'gtsvector'))
),
candidates AS (
    SELECT format_type(u.oid, NULL)::text AS value, u.kind AS detail, u.typname, u.nspname FROM usable u
    UNION ALL
    SELECT format_type(u.typarray, NULL)::text, 'array', u.typname, u.nspname FROM usable u WHERE u.typarray <> 0
)
SELECT value, detail
FROM candidates
WHERE $1::text IS NULL OR strpos(lower(value), lower($1::text)) > 0 OR strpos(lower(typname), lower($1::text)) > 0
ORDER BY (left(lower(value), length(coalesce($1::text, ''))) = lower(coalesce($1::text, ''))
          OR left(lower(typname), length(coalesce($1::text, ''))) = lower(coalesce($1::text, ''))) DESC,
         (nspname = 'pg_catalog') DESC, (detail = 'array'), length(value), value
LIMIT 200
