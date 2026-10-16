-- Function/Procedure başlığı (PG 12–13: prosqlbody yok). Alanlar load_function.sql ile aynı.
-- kind: prokind (f/p/a/w). ret_*: dönüş sınıfı (TABLE dönüşü argüman sorgusundaki 't' modlarından çıkarılır).
-- cost/rows: float4 (Rust'ta üstelsiz ondalık metne çevrilir). settings: proconfig ("ad=değer").
-- Salt okunurluk girdileri: extension (pg_depend deptype 'e'), support (prosupport, PG 12+), has_transform
-- (protrftypes), sql_body (prosqlbody, PG 14+ BEGIN ATOMIC), is_system.
SELECT n.nspname::text AS schema,
       p.proname::text AS name,
       pg_get_userbyid(p.proowner)::text AS owner,
       obj_description(p.oid, 'pg_proc') AS comment,
       l.lanname::text AS language,
       p.prokind::text AS kind,
       p.proretset AS retset,
       format_type(p.prorettype, NULL) AS ret_type,
       p.prorettype = 'pg_catalog.trigger'::pg_catalog.regtype AS ret_trigger,
       p.prorettype = 'pg_catalog.void'::pg_catalog.regtype AS ret_void,
       p.prosrc AS body,
       p.provolatile::text AS volatility,
       p.proisstrict AS strict,
       p.prosecdef AS security_definer,
       p.proleakproof AS leakproof,
       p.proparallel::text AS parallel,
       p.procost AS cost,
       p.prorows AS rows,
       COALESCE(p.proconfig, '{}')::text[] AS settings,
       (SELECT e.extname::text FROM pg_depend d JOIN pg_extension e ON e.oid = d.refobjid
        WHERE d.classid = 'pg_catalog.pg_proc'::pg_catalog.regclass AND d.objid = p.oid
          AND d.refclassid = 'pg_catalog.pg_extension'::pg_catalog.regclass AND d.deptype = 'e'
        ORDER BY 1 LIMIT 1) AS extension,
       CASE WHEN p.prosupport <> 0 THEN p.prosupport::pg_catalog.regproc::text END AS support,
       p.protrftypes IS NOT NULL AS has_transform,
       false AS sql_body,
       (left(n.nspname, 3) = 'pg_' OR n.nspname = 'information_schema') AS is_system
FROM pg_proc p
JOIN pg_namespace n ON n.oid = p.pronamespace
JOIN pg_language l ON l.oid = p.prolang
WHERE p.oid = $1
