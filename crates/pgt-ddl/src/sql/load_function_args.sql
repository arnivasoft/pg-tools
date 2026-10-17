-- Function/Procedure argümanları (tüm sürümler). $1 = pg_proc oid'i. Tüm argümanlar (proallargtypes; yalnız IN
-- argümanlı fonksiyonda NULL'dur → proargtypes) konum sırasıyla. mode: proargmodes öğesi (NULL → 'i'; 'o', 'b' INOUT,
-- 'v' VARIADIC, 't' TABLE kolonu). name: boş ad → NULL. type_name: format_type (değiştiricisiz).
-- default: pg_get_function_arg_default(oid, n) — n tüm argümanlarda 1 tabanlı konum; giriş olmayan ya da varsayılansız
-- argümanda NULL. proargtypes (oidvector) alt sınırı 0'dır; konum WITH ORDINALITY'den alınır.
SELECT a.n::int4 AS pos,
       COALESCE(p.proargmodes[a.n], 'i')::text AS mode,
       NULLIF(p.proargnames[a.n], '') AS name,
       format_type(a.t, NULL) AS type_name,
       pg_get_function_arg_default(p.oid, a.n::int4) AS "default"
FROM pg_proc p,
     unnest(COALESCE(p.proallargtypes, p.proargtypes::oid[])) WITH ORDINALITY AS a(t, n)
WHERE p.oid = $1
ORDER BY a.n
