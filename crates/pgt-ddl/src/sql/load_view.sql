-- Görünüm başlığı. $1 = görünüm (pg_class) oid'i; yalnızca relkind 'v' (başka tür → satır yok → NotFound).
-- query: pg_get_viewdef(.., true) (pretty; sondaki ';' Rust'ta soyulur). options: reloptions ("k=v"; check_option,
-- security_barrier, PG 15+ security_invoker — PG 10–14'te anahtar zaten bulunmaz, tek dosya yeterli).
-- triggers/rules: yeniden kurmada kaybolacak modellenmeyen ekler (iç tetikleyiciler ve '_RETURN' kuralı hariç), ad sırasıyla.
SELECT n.nspname::text AS schema,
       c.relname::text AS name,
       pg_get_userbyid(c.relowner)::text AS owner,
       pg_get_viewdef(c.oid, true) AS query,
       c.relpersistence::text AS persistence,
       COALESCE(c.reloptions, '{}')::text[] AS options,
       obj_description(c.oid, 'pg_class') AS comment,
       ARRAY(SELECT t.tgname::text FROM pg_trigger t
             WHERE t.tgrelid = c.oid AND NOT t.tgisinternal
             ORDER BY t.tgname::text COLLATE "C")::text[] AS triggers,
       ARRAY(SELECT r.rulename::text FROM pg_rewrite r
             WHERE r.ev_class = c.oid AND r.rulename <> '_RETURN'
             ORDER BY r.rulename::text COLLATE "C")::text[] AS rules,
       (left(n.nspname, 3) = 'pg_' OR n.nspname = 'information_schema') AS is_system
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.oid = $1 AND c.relkind = 'v'
