SELECT pg_get_indexdef(x.indexrelid) || ';' AS def
FROM pg_index x
JOIN pg_class ic ON ic.oid = x.indexrelid
WHERE x.indrelid = $1
  AND NOT EXISTS (SELECT 1 FROM pg_constraint k WHERE k.conindid = x.indexrelid AND k.contype IN ('p', 'u', 'x'))
  AND NOT EXISTS (SELECT 1 FROM pg_inherits ih WHERE ih.inhrelid = x.indexrelid)
ORDER BY ic.relname COLLATE "C"
