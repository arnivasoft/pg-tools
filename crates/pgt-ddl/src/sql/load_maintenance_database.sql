-- Bakım hedefi: veritabanı (bağlı veritabanında okunur). $1 = pg_database oid'i (yoksa satır yok → NotFound).
-- `is_current`: istenen veritabanı bağlı olunan veritabanı mı (değilse yükleyici `Invalid` döner). Ad ham metin.
SELECT NULL::text AS schema,
       d.datname::text AS name,
       'd'::text AS relkind,
       NULL::text AS table_name,
       false AS is_system,
       false AS is_temp,
       ARRAY[]::text[] AS indexes,
       NULL::text AS cluster_index,
       d.datname = current_database() AS is_current
FROM pg_database d
WHERE d.oid = $1
