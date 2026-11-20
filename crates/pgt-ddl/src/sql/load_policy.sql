-- Politika (satır güvenliği). $1 = pg_policy oid'i (yoksa satır yok → NotFound).
-- command: polcmd ('*' = all, r/a/w/d). roles: polroles → rol adları, ada göre COLLATE "C" sıralı; polroles = {0}
-- (PUBLIC) pg_roles'ta karşılığı olmadığından boş liste olur. using/with_check: pg_get_expr(…, polrelid) metni (yükleyici
-- ve üretici aynı metni karşılaştırır → alter(yüklü, yüklü) boş). table_rls: tablonun güncel bayrakları. extension:
-- tablo eklentiye bağlıysa (deptype 'e') salt okunur. relkind: r/p dışı beklenmez (politika yalnız tabloda olur).
SELECT n.nspname::text AS schema,
       c.relname::text AS table_name,
       p.polname::text AS name,
       p.polcmd::text AS command,
       p.polpermissive AS permissive,
       ARRAY(SELECT r.rolname::text FROM pg_roles r WHERE r.oid = ANY (p.polroles) ORDER BY r.rolname::text COLLATE "C")::text[] AS roles,
       pg_get_expr(p.polqual, p.polrelid)::text AS using_expr,
       pg_get_expr(p.polwithcheck, p.polrelid)::text AS check_expr,
       obj_description(p.oid, 'pg_policy') AS comment,
       c.relrowsecurity AS rls_enabled,
       c.relforcerowsecurity AS rls_forced,
       (SELECT e.extname::text FROM pg_depend d JOIN pg_extension e ON e.oid = d.refobjid
         WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.refclassid = 'pg_extension'::regclass
           AND d.deptype = 'e' LIMIT 1) AS extension,
       c.relkind::text AS relkind,
       (left(n.nspname, 3) = 'pg_' OR n.nspname = 'information_schema') AS is_system
FROM pg_policy p
JOIN pg_class c ON c.oid = p.polrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE p.oid = $1
