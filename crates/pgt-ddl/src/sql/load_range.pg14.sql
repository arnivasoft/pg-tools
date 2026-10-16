-- Range gövdesi (PG 14+). $1 = pg_type (range) oid'i. Alanlar load_range.sql ile aynı; multirange: rngmultitypid'in
-- adı (aynı şemada; Rust otomatik varsayılan adla karşılaştırır, eşitse None).
SELECT format_type(r.rngsubtype, NULL) AS subtype,
       CASE WHEN oc.opcdefault THEN NULL ELSE oc.opcname::text END AS opclass,
       CASE WHEN r.rngcollation <> 0 AND r.rngcollation <> st.typcollation THEN co.collname::text END AS collation,
       CASE WHEN r.rngcanonical <> 0 THEN r.rngcanonical::pg_catalog.regproc::text END AS canonical,
       CASE WHEN r.rngsubdiff <> 0 THEN r.rngsubdiff::pg_catalog.regproc::text END AS subtype_diff,
       (SELECT m.typname::text FROM pg_type m WHERE m.oid = r.rngmultitypid) AS multirange
FROM pg_range r
JOIN pg_type st ON st.oid = r.rngsubtype
JOIN pg_opclass oc ON oc.oid = r.rngsubopc
LEFT JOIN pg_collation co ON co.oid = r.rngcollation
WHERE r.rngtypid = $1
