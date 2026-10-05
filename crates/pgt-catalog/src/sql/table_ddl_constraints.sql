SELECT quote_ident(k.conname) AS name, pg_get_constraintdef(k.oid, true) AS def
FROM pg_constraint k
WHERE k.conrelid = $1 AND k.contype IN ('p', 'u', 'c', 'x', 'f') AND k.conislocal
ORDER BY CASE k.contype WHEN 'p' THEN 0 WHEN 'u' THEN 1 WHEN 'c' THEN 2 WHEN 'x' THEN 3 ELSE 4 END, k.conname COLLATE "C"
