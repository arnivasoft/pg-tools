-- Tablespace (sunucu kapsamlı; bakım bağlantısında çalışır). $1 = pg_tablespace oid'i.
-- location: pg_tablespace_location; pg_default/pg_global için boş dizge. options: spcoptions "ad=değer" öğeleri
-- (değerler sunucunun sakladığı METİN; ayrıştırma Rust'ta). comment: pg_shdescription.
SELECT t.spcname::text AS name,
       pg_get_userbyid(t.spcowner)::text AS owner,
       pg_tablespace_location(t.oid)::text AS location,
       COALESCE(t.spcoptions, '{}')::text[] AS options,
       shobj_description(t.oid, 'pg_tablespace') AS comment,
       t.spcname IN ('pg_default', 'pg_global') AS is_system
FROM pg_tablespace t
WHERE t.oid = $1
