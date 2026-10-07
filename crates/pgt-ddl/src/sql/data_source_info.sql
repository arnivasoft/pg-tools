-- Veri düzenleyici kaynak başlığı. $1 = ilişki (pg_class) oid'i. relkind ham döner; r p v m f dışı Rust'ta reddedilir.
-- Tablo düzeyi yetkiler has_table_privilege ile (geçerli kullanıcı; kolon düzeyi yetkiler kolon sorgusunda).
SELECT n.nspname::text AS schema,
       c.relname::text AS name,
       c.relkind::text AS relkind,
       has_table_privilege(c.oid, 'SELECT') AS can_select,
       has_table_privilege(c.oid, 'INSERT') AS can_insert,
       has_table_privilege(c.oid, 'UPDATE') AS can_update,
       has_table_privilege(c.oid, 'DELETE') AS can_delete
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.oid = $1
