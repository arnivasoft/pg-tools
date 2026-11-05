-- Şema nesnelerini listeler (PG 13+). $1 = şema adı.
-- Kolonlar: kind, oid, name, table_name, signature, is_partition, deps (görünüm/MV bağımlı ilişki oid'leri).
-- Uzantı üyeleri (pg_depend 'e') ve bunların tablolarının indeks/trigger'ları listeye girmez.
SELECT kind, oid, name, table_name, signature, is_partition, deps
FROM (
    -- İlişkiler: tablo, görünüm, materialized view, sequence (kimlik dizileri hariç).
    SELECT CASE c.relkind
               WHEN 'v' THEN 'view'
               WHEN 'm' THEN 'materialized_view'
               WHEN 'S' THEN 'sequence'
               ELSE 'table'
           END AS kind,
           c.oid AS oid,
           c.relname::pg_catalog.text AS name,
           NULL::pg_catalog.text AS table_name,
           NULL::pg_catalog.text AS signature,
           c.relispartition AS is_partition,
           CASE WHEN c.relkind IN ('v', 'm')
                THEN ARRAY(SELECT DISTINCT d.refobjid
                           FROM pg_catalog.pg_rewrite r
                           JOIN pg_catalog.pg_depend d
                             ON d.classid = 'pg_catalog.pg_rewrite'::pg_catalog.regclass
                            AND d.objid = r.oid
                            AND d.refclassid = 'pg_catalog.pg_class'::pg_catalog.regclass
                            AND d.refobjid <> c.oid
                           WHERE r.ev_class = c.oid
                           ORDER BY 1)
                ELSE '{}'::pg_catalog.oid[]
           END AS deps
    FROM pg_catalog.pg_class c
    JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = $1::pg_catalog.text
      AND c.relkind IN ('r', 'p', 'v', 'm', 'S')
      AND NOT (c.relkind = 'S' AND EXISTS (
            SELECT 1 FROM pg_catalog.pg_depend d
            WHERE d.classid = 'pg_catalog.pg_class'::pg_catalog.regclass AND d.objid = c.oid
              AND d.refclassid = 'pg_catalog.pg_class'::pg_catalog.regclass AND d.deptype = 'i'))
      AND NOT EXISTS (
            SELECT 1 FROM pg_catalog.pg_depend d
            WHERE d.classid = 'pg_catalog.pg_class'::pg_catalog.regclass AND d.objid = c.oid AND d.deptype = 'e')

    UNION ALL

    -- İndeksler (kısıt indeksleri ve bölüm klonları hariç).
    SELECT 'index', ic.oid, ic.relname::pg_catalog.text, t.relname::pg_catalog.text, NULL::pg_catalog.text,
           false, '{}'::pg_catalog.oid[]
    FROM pg_catalog.pg_index i
    JOIN pg_catalog.pg_class ic ON ic.oid = i.indexrelid
    JOIN pg_catalog.pg_class t ON t.oid = i.indrelid
    JOIN pg_catalog.pg_namespace n ON n.oid = t.relnamespace
    WHERE n.nspname = $1::pg_catalog.text
      AND t.relkind IN ('r', 'p', 'm')
      AND NOT EXISTS (
            SELECT 1 FROM pg_catalog.pg_constraint k
            WHERE k.conindid = i.indexrelid AND k.contype IN ('p', 'u', 'x'))
      AND NOT EXISTS (
            SELECT 1 FROM pg_catalog.pg_inherits h WHERE h.inhrelid = i.indexrelid)
      AND NOT EXISTS (
            SELECT 1 FROM pg_catalog.pg_depend d
            WHERE d.classid = 'pg_catalog.pg_class'::pg_catalog.regclass AND d.objid = t.oid AND d.deptype = 'e')

    UNION ALL

    -- Trigger'lar (iç trigger'lar ve bölüm klonları hariç).
    SELECT 'trigger', g.oid, g.tgname::pg_catalog.text, t.relname::pg_catalog.text, NULL::pg_catalog.text,
           false, '{}'::pg_catalog.oid[]
    FROM pg_catalog.pg_trigger g
    JOIN pg_catalog.pg_class t ON t.oid = g.tgrelid
    JOIN pg_catalog.pg_namespace n ON n.oid = t.relnamespace
    WHERE n.nspname = $1::pg_catalog.text
      AND NOT g.tgisinternal
      AND g.tgparentid = 0
      AND t.relkind IN ('r', 'p')
      AND NOT EXISTS (
            SELECT 1 FROM pg_catalog.pg_depend d
            WHERE d.classid = 'pg_catalog.pg_class'::pg_catalog.regclass AND d.objid = t.oid AND d.deptype = 'e')

    UNION ALL

    -- Rutinler (aggregate ve tipin otomatik işlevleri - range kurucuları - hariç).
    SELECT CASE p.prokind WHEN 'p' THEN 'procedure' ELSE 'function' END, p.oid, p.proname::pg_catalog.text,
           NULL::pg_catalog.text, pg_catalog.oidvectortypes(p.proargtypes)::pg_catalog.text,
           false, '{}'::pg_catalog.oid[]
    FROM pg_catalog.pg_proc p
    JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
    WHERE n.nspname = $1::pg_catalog.text
      AND p.prokind IN ('f', 'w', 'p')
      AND NOT EXISTS (
            SELECT 1 FROM pg_catalog.pg_depend d
            WHERE d.classid = 'pg_catalog.pg_proc'::pg_catalog.regclass AND d.objid = p.oid AND d.deptype IN ('e', 'i'))

    UNION ALL

    -- Tipler: enum, range, serbest composite ve domain (dizi, multirange ve tablo satır tipleri düşer).
    SELECT CASE ty.typtype WHEN 'd' THEN 'domain' ELSE 'type' END, ty.oid, ty.typname::pg_catalog.text,
           NULL::pg_catalog.text, NULL::pg_catalog.text, false, '{}'::pg_catalog.oid[]
    FROM pg_catalog.pg_type ty
    JOIN pg_catalog.pg_namespace n ON n.oid = ty.typnamespace
    LEFT JOIN pg_catalog.pg_class tc ON tc.oid = ty.typrelid
    WHERE n.nspname = $1::pg_catalog.text
      AND (ty.typtype IN ('e', 'r', 'd') OR (ty.typtype = 'c' AND tc.relkind = 'c'))
      AND NOT EXISTS (
            SELECT 1 FROM pg_catalog.pg_depend d
            WHERE d.classid = 'pg_catalog.pg_type'::pg_catalog.regclass AND d.objid = ty.oid AND d.deptype = 'e')
) o
ORDER BY kind COLLATE "C", table_name COLLATE "C", name COLLATE "C"
