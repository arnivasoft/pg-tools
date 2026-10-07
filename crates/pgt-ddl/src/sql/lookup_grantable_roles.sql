-- Üyelik için roller: tüm roller (önceden tanımlı pg_* dahil), ad sırasıyla. pg_database_owner (PG 14+) hariç: üyeliği
-- örtüktür, açık GRANT reddedilir.
SELECT rolname::text AS value,
       CASE WHEN left(rolname, 3) = 'pg_' THEN 'önceden tanımlı'
            WHEN rolsuper THEN 'superuser'
            ELSE NULL END AS detail
FROM pg_roles
WHERE rolname <> 'pg_database_owner'
ORDER BY rolname::text COLLATE "C"
