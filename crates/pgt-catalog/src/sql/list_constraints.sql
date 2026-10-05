SELECT k.oid, k.conname::text AS name,
       CASE k.contype WHEN 'p' THEN 'primary key' WHEN 'f' THEN 'foreign key' WHEN 'u' THEN 'unique'
                      WHEN 'c' THEN 'check' WHEN 'x' THEN 'exclusion' END AS detail,
       true AS leaf, k.contype = 'p' AS primary_key
FROM pg_constraint k
WHERE k.conrelid = $1 AND k.contype IN ('p', 'u', 'c', 'x', 'f')
ORDER BY CASE k.contype WHEN 'p' THEN 0 WHEN 'u' THEN 1 WHEN 'c' THEN 2 WHEN 'x' THEN 3 ELSE 4 END,
         k.conname::text COLLATE "C"
