-- Domain başlığı (tüm sürümler). $1 = pg_type oid'i (yoksa satır yok → NotFound; typtype <> 'd' → Invalid).
-- base_type: format_type (değiştiricili). collation yalnızca temel tipin varsayılanından FARKLIysa (şema niteliksiz ad).
-- default: pg_get_expr(typdefaultbin, 0, true) (Rust'ta tamamını saran dış parantez soyulur).
SELECT n.nspname::text AS schema,
       t.typname::text AS name,
       pg_get_userbyid(t.typowner)::text AS owner,
       obj_description(t.oid, 'pg_type') AS comment,
       t.typtype::text AS typtype,
       format_type(t.typbasetype, t.typtypmod) AS base_type,
       CASE WHEN t.typcollation <> COALESCE(bt.typcollation, 0) THEN co.collname::text END AS collation,
       pg_get_expr(t.typdefaultbin, 0, true) AS "default",
       t.typnotnull AS not_null,
       (SELECT e.extname::text FROM pg_depend d JOIN pg_extension e ON e.oid = d.refobjid
        WHERE d.classid = 'pg_catalog.pg_type'::pg_catalog.regclass AND d.objid = t.oid
          AND d.refclassid = 'pg_catalog.pg_extension'::pg_catalog.regclass AND d.deptype = 'e'
        ORDER BY 1 LIMIT 1) AS extension,
       (left(n.nspname, 3) = 'pg_' OR n.nspname = 'information_schema') AS is_system
FROM pg_type t
JOIN pg_namespace n ON n.oid = t.typnamespace
LEFT JOIN pg_type bt ON bt.oid = t.typbasetype
LEFT JOIN pg_collation co ON co.oid = t.typcollation
WHERE t.oid = $1
