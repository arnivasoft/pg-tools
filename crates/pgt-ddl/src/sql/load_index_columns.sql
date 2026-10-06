-- İndeks kolonları (anahtar + INCLUDE), sırayla. $1 = indeks oid'i.
-- key_text: düz kolonda ad, ifadede pg_get_indexdef(.., k, true). opclass yalnızca varsayılan DEĞİLSE;
-- collation yalnızca kolonun/ifadenin doğal collation'ından FARKLIysa (100 = "default").
SELECT k.n::int4 AS pos,
       i.indkey[k.n - 1]::int4 AS attnum,
       CASE WHEN i.indkey[k.n - 1] = 0 THEN pg_get_indexdef(i.indexrelid, k.n, true) ELSE a.attname::text END AS key_text,
       CASE WHEN oc.opcdefault THEN NULL ELSE oc.opcname::text END AS opclass,
       CASE WHEN i.indcollation[k.n - 1] = 0 THEN NULL
            WHEN i.indkey[k.n - 1] > 0 AND i.indcollation[k.n - 1] = a.attcollation THEN NULL
            WHEN i.indkey[k.n - 1] = 0 AND i.indcollation[k.n - 1] = 100 THEN NULL
            ELSE co.collname::text END AS collation,
       i.indoption[k.n - 1]::int4 AS options
FROM pg_index i
CROSS JOIN LATERAL generate_series(1, i.indnatts::int4) AS k(n)
LEFT JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = i.indkey[k.n - 1] AND i.indkey[k.n - 1] > 0
LEFT JOIN pg_opclass oc ON oc.oid = i.indclass[k.n - 1]
LEFT JOIN pg_collation co ON co.oid = i.indcollation[k.n - 1]
WHERE i.indexrelid = $1
ORDER BY k.n
