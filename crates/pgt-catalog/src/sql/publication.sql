SELECT p.oid, p.pubname::text AS name, quote_ident(p.pubname) AS qname,
       pg_get_userbyid(p.pubowner)::text AS owner, quote_ident(pg_get_userbyid(p.pubowner)) AS qowner,
       obj_description(p.oid, 'pg_publication') AS comment,
       p.puballtables AS all_tables, p.pubinsert AS ins, p.pubupdate AS upd, p.pubdelete AS del,
       (SELECT count(*) FROM pg_publication_rel r WHERE r.prpubid = p.oid)::int4 AS table_count
FROM pg_publication p
WHERE p.oid = $1
