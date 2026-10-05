SELECT x.indexrelid AS oid, c.relname::text AS name,
       concat_ws(', ', am.amname, CASE WHEN x.indisprimary THEN 'primary key' WHEN x.indisunique THEN 'unique' END) AS detail,
       true AS leaf, x.indisprimary AS primary_key
FROM pg_index x
JOIN pg_class c ON c.oid = x.indexrelid
JOIN pg_am am ON am.oid = c.relam
WHERE x.indrelid = $1
ORDER BY c.relname::text COLLATE "C"
