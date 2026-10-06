-- Şema tanımı: ad, sahip, yorum. $1 = şema oid'i.
SELECT n.nspname::text AS name,
       pg_get_userbyid(n.nspowner)::text AS owner,
       obj_description(n.oid, 'pg_namespace') AS comment,
       (left(n.nspname, 3) = 'pg_' OR n.nspname = 'information_schema') AS is_system
FROM pg_namespace n
WHERE n.oid = $1
