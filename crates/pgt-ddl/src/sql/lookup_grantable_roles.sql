-- Üyelik için roller: tüm roller (önceden tanımlı pg_* dahil), ad sırasıyla.
SELECT rolname::text AS value,
       CASE WHEN left(rolname, 3) = 'pg_' THEN 'önceden tanımlı'
            WHEN rolsuper THEN 'superuser'
            ELSE NULL END AS detail
FROM pg_roles
ORDER BY rolname::text COLLATE "C"
