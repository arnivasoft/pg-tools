SELECT p.oid, p.pubname::text AS name,
       CASE WHEN p.puballtables THEN 'all tables'
            ELSE (SELECT count(*) FROM pg_publication_rel r WHERE r.prpubid = p.oid)::text || ' tablo'
                 || COALESCE((SELECT ', ' || count(*)::text || ' şema' FROM pg_publication_namespace n WHERE n.pnpubid = p.oid HAVING count(*) > 0), '') END AS detail,
       true AS leaf
FROM pg_publication p
ORDER BY p.pubname::text COLLATE "C"
