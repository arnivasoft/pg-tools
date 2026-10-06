-- Sequence tanımı (pg_sequence: PG 10+). $1 = sequence oid'i.
-- OWNED BY yalnızca normal bağımlılıktan ('a'; kimlik/IDENTITY 'i' değildir) ve kolon düzeyindendir.
SELECT n.nspname::text AS schema,
       c.relname::text AS name,
       pg_get_userbyid(c.relowner)::text AS owner,
       format_type(s.seqtypid, NULL)::text AS data_type,
       s.seqstart::int8 AS start,
       s.seqincrement::int8 AS increment,
       s.seqmin::int8 AS min_value,
       s.seqmax::int8 AS max_value,
       s.seqcache::int8 AS cache,
       s.seqcycle AS cycle,
       on_.nspname::text AS owned_schema,
       oc.relname::text AS owned_table,
       oa.attname::text AS owned_column,
       EXISTS (SELECT 1 FROM pg_depend di WHERE di.classid = 'pg_class'::regclass AND di.objid = c.oid
               AND di.refclassid = 'pg_class'::regclass AND di.deptype = 'i') AS is_identity,
       obj_description(c.oid, 'pg_class') AS comment,
       (left(n.nspname, 3) = 'pg_' OR n.nspname = 'information_schema') AS is_system
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_sequence s ON s.seqrelid = c.oid
LEFT JOIN pg_depend d ON d.classid = 'pg_class'::regclass AND d.objid = c.oid
     AND d.refclassid = 'pg_class'::regclass AND d.deptype = 'a' AND d.refobjsubid > 0
LEFT JOIN pg_class oc ON oc.oid = d.refobjid
LEFT JOIN pg_namespace on_ ON on_.oid = oc.relnamespace
LEFT JOIN pg_attribute oa ON oa.attrelid = d.refobjid AND oa.attnum = d.refobjsubid
WHERE c.oid = $1 AND c.relkind = 'S'
