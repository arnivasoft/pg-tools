-- Roller (sistem rolleri pg_ önekliler hariç).
SELECT rolname::text AS value,
       CASE WHEN rolsuper THEN 'superuser' ELSE NULL END AS detail
FROM pg_roles
WHERE left(rolname, 3) <> 'pg_'
ORDER BY rolname
