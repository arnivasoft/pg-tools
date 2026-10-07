-- CREATE DATABASE … TEMPLATE için veritabanları: şablon ya da bağlantıya açık olanlar (template0 dahil); şablonlar önce.
SELECT datname::text AS value,
       CASE WHEN datistemplate THEN 'şablon' ELSE NULL END AS detail
FROM pg_database
WHERE datistemplate OR datallowconn
ORDER BY datistemplate DESC, datname::text COLLATE "C"
