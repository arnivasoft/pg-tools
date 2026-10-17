-- Type başlığı (tüm sürümler). $1 = pg_type oid'i (yoksa satır yok → NotFound).
-- typtype: e enum, c composite (relkind 'c' bağımsız composite; başka relkind tablo/görünüm satır tipi), r range,
-- m multirange (PG 14+), b base (category 'A' dizi), p pseudo (typisdefined = false → shell), d domain (Type değil).
-- Salt okunurluk girdileri: extension (pg_depend deptype 'e'), is_system.
SELECT n.nspname::text AS schema,
       t.typname::text AS name,
       pg_get_userbyid(t.typowner)::text AS owner,
       obj_description(t.oid, 'pg_type') AS comment,
       t.typtype::text AS typtype,
       t.typcategory::text AS category,
       t.typisdefined AS defined,
       (SELECT r.relkind::text FROM pg_class r WHERE r.oid = t.typrelid) AS relkind,
       (SELECT e.extname::text FROM pg_depend d JOIN pg_extension e ON e.oid = d.refobjid
        WHERE d.classid = 'pg_catalog.pg_type'::pg_catalog.regclass AND d.objid = t.oid
          AND d.refclassid = 'pg_catalog.pg_extension'::pg_catalog.regclass AND d.deptype = 'e'
        ORDER BY 1 LIMIT 1) AS extension,
       (left(n.nspname, 3) = 'pg_' OR n.nspname = 'information_schema') AS is_system
FROM pg_type t
JOIN pg_namespace n ON n.oid = t.typnamespace
WHERE t.oid = $1
