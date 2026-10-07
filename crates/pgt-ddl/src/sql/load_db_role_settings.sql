-- Rolün ayarları (pg_db_role_setting, setrole = $1): database = NULL → tüm veritabanları (ALTER ROLE … SET),
-- dolu → ALTER ROLE … IN DATABASE (desteklenmeyen). Öğeler "ad=değer"; ad küçültme ve sıralama Rust'ta.
SELECT CASE WHEN s.setdatabase = 0 THEN NULL
            ELSE COALESCE((SELECT d.datname::text FROM pg_database d WHERE d.oid = s.setdatabase),
                          s.setdatabase::text) END AS database,
       s.setconfig::text[] AS settings
FROM pg_db_role_setting s
WHERE s.setrole = $1
ORDER BY s.setdatabase
