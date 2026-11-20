-- Bölüm (partition; PG 14+). $1 = bölüm tablosunun (çocuk) pg_class oid'i; bölüm değilse ya da üstü bölümlü değilse satır yok
-- (NotFound). strat: partstrat ('r' range, 'l' list, 'h' hash); key_count: partnatts; bound: pg_get_expr(relpartbound)
-- metni (Rust ayrıştırır). detach_pending: inhdetachpending (yarım kalmış DETACH CONCURRENTLY). extension: çocuk ya da üst tablo
-- eklentiye bağlıysa (deptype 'e') eklenti adı → salt okunur.
SELECT n.nspname::text AS schema,
       c.relname::text AS name,
       pn.nspname::text AS parent_schema,
       p.relname::text AS parent_table,
       pt.partstrat::text AS strat,
       pt.partnatts::int4 AS key_count,
       pg_get_expr(c.relpartbound, c.oid)::text AS bound,
       i.inhdetachpending AS detach_pending,
       (SELECT e.extname::text FROM pg_depend d JOIN pg_extension e ON e.oid = d.refobjid
         WHERE d.classid = 'pg_class'::regclass AND d.objid IN (c.oid, p.oid) AND d.refclassid = 'pg_extension'::regclass
           AND d.deptype = 'e' LIMIT 1) AS extension
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_inherits i ON i.inhrelid = c.oid
JOIN pg_class p ON p.oid = i.inhparent AND p.relkind = 'p'
JOIN pg_namespace pn ON pn.oid = p.relnamespace
JOIN pg_partitioned_table pt ON pt.partrelid = p.oid
WHERE c.oid = $1 AND c.relispartition
