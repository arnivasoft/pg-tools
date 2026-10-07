-- Veritabanı tanımı (PG 15–16: datlocprovider, daticulocale). $1 = veritabanı (pg_database) oid'i.
-- encoding: pg_encoding_to_char (SERVER_ENCODINGS yazımı). tablespace: her zaman dolu (dattablespace).
-- settings: veritabanı genelindeki ayarlar (pg_db_role_setting, setrole = 0) "ad=değer" öğeleri; ad küçültme ve
-- sıralama Rust'ta. Bakım bağlantısında çalışır (paylaşılan kataloglar; süper kullanıcı gerekmez).
SELECT d.datname::text AS name,
       pg_get_userbyid(d.datdba)::text AS owner,
       pg_encoding_to_char(d.encoding)::text AS encoding,
       d.datlocprovider::text AS locale_provider,
       d.datcollate::text AS lc_collate,
       d.datctype::text AS lc_ctype,
       d.daticulocale::text AS provider_locale,
       t.spcname::text AS tablespace,
       d.datconnlimit AS connection_limit,
       d.datistemplate AS is_template,
       d.datallowconn AS allow_connections,
       COALESCE((SELECT s.setconfig FROM pg_db_role_setting s WHERE s.setdatabase = d.oid AND s.setrole = 0),
                '{}')::text[] AS settings,
       shobj_description(d.oid, 'pg_database') AS comment
FROM pg_database d
JOIN pg_tablespace t ON t.oid = d.dattablespace
WHERE d.oid = $1
