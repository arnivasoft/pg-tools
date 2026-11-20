SELECT p.oid, p.pubname::text AS name,
       CASE WHEN p.puballtables THEN 'all tables'
            ELSE (SELECT count(*) FROM pg_publication_rel r WHERE r.prpubid = p.oid)::text || ' tablo' END AS detail,
       true AS leaf
FROM pg_publication p
ORDER BY p.pubname::text COLLATE "C"
