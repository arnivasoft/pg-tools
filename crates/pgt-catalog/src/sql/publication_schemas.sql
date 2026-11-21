-- PG 15+: FOR TABLES IN SCHEMA.
SELECT n.nspname::text AS name, quote_ident(n.nspname) AS qname
FROM pg_publication_namespace pn
JOIN pg_namespace n ON n.oid = pn.pnnspid
WHERE pn.pnpubid = $1
ORDER BY n.nspname::text COLLATE "C"
