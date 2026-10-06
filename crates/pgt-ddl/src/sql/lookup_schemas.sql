-- Kullanıcı şemaları (pg_* ve information_schema hariç); ayrıntı = sahip.
SELECT n.nspname::text AS value, pg_get_userbyid(n.nspowner)::text AS detail
FROM pg_namespace n
WHERE left(n.nspname, 3) <> 'pg_' AND n.nspname <> 'information_schema'
ORDER BY n.nspname
