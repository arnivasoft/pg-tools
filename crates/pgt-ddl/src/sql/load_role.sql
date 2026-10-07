-- Rol tanımı. $1 = rol oid'i. pg_roles (pg_authid DEĞİL): süper kullanıcı olmayan bağlantıda da çalışır; parola
-- sütunu (rolpassword) hiç seçilmez — parola niyeti yükleyicide her zaman "keep".
-- valid_until: oturum saat diliminden bağımsız sabit UTC biçimi YYYY-MM-DDTHH:MM:SS[.ffffff]Z (kesir yalnızca sıfır
-- değilse, 6 hane); sonsuz değer olduğu gibi ('infinity'). Ayarlar ayrı sorguda (load_db_role_settings.sql).
SELECT r.rolname::text AS name,
       r.rolcanlogin AS login,
       r.rolsuper AS superuser,
       r.rolcreatedb AS createdb,
       r.rolcreaterole AS createrole,
       r.rolinherit AS inherit,
       r.rolreplication AS replication,
       r.rolbypassrls AS bypassrls,
       r.rolconnlimit AS connection_limit,
       CASE WHEN r.rolvaliduntil IS NULL THEN NULL
            WHEN NOT isfinite(r.rolvaliduntil) THEN r.rolvaliduntil::text
            ELSE to_char(r.rolvaliduntil AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS')
                 || CASE WHEN extract(microseconds FROM r.rolvaliduntil)::bigint % 1000000 <> 0
                         THEN to_char(r.rolvaliduntil AT TIME ZONE 'UTC', '.US') ELSE '' END
                 || 'Z'
       END AS valid_until,
       shobj_description(r.oid, 'pg_authid') AS comment,
       (left(r.rolname, 3) = 'pg_') AS is_system
FROM pg_roles r
WHERE r.oid = $1
