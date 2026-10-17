-- Trigger (PG 10–12; tgparentid yok → is_clone = false). $1 = pg_trigger oid'i (yoksa satır yok → NotFound).
-- tgtype bitleri Rust'ta çözülür (ROW=1, BEFORE=2, INSERT=4, DELETE=8, UPDATE=16, TRUNCATE=32, INSTEAD=64).
-- update_columns: tgattr saklandığı sırayla kolon adları. args: tgargs ham baytları (her argüman \000 ile biter;
-- tgnargs adet). def: pg_get_triggerdef(oid, true) — WHEN koşulu buradan ayrıştırılır (pg_get_expr, tgqual'daki
-- OLD/NEW değişkenlerini çözemez). Constraint: tgconstraint <> 0; ertelenebilirlik pg_constraint'ten (yoksa
-- pg_trigger). from_table: tgconstrrelid (FROM başvurulan tablo; modellenmez). relkind: r/p dışı (görünüm, yabancı
-- tablo) salt okunur. encoding: tgargs baytlarının kodlaması.
SELECT n.nspname::text AS schema,
       c.relname::text AS table_name,
       c.relkind::text AS relkind,
       t.tgname::text AS name,
       t.tgtype::int4 AS tgtype,
       ARRAY(SELECT a.attname::text
             FROM unnest(t.tgattr::int2[]) WITH ORDINALITY AS k(attnum, n)
             JOIN pg_attribute a ON a.attrelid = t.tgrelid AND a.attnum = k.attnum
             ORDER BY k.n)::text[] AS update_columns,
       t.tgnargs::int4 AS nargs,
       t.tgargs AS args,
       pg_get_triggerdef(t.oid, true) AS def,
       t.tgoldtable::text AS old_table,
       t.tgnewtable::text AS new_table,
       t.tgconstraint <> 0 AS is_constraint,
       COALESCE(con.condeferrable, t.tgdeferrable) AS deferrable,
       COALESCE(con.condeferred, t.tginitdeferred) AS initially_deferred,
       CASE WHEN t.tgconstrrelid <> 0 THEN t.tgconstrrelid::pg_catalog.regclass::text END AS from_table,
       t.tgenabled::text AS enabled,
       fn.nspname::text AS function_schema,
       p.proname::text AS function_name,
       obj_description(t.oid, 'pg_trigger') AS comment,
       t.tgisinternal AS is_internal,
       false AS is_clone,
       current_setting('server_encoding') AS encoding,
       (left(n.nspname, 3) = 'pg_' OR n.nspname = 'information_schema') AS is_system
FROM pg_trigger t
JOIN pg_class c ON c.oid = t.tgrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_proc p ON p.oid = t.tgfoid
JOIN pg_namespace fn ON fn.oid = p.pronamespace
LEFT JOIN pg_constraint con ON con.oid = t.tgconstraint
WHERE t.oid = $1
