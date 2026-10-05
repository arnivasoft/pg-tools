SELECT pg_get_triggerdef(t.oid, true) || ';' AS def,
       CASE t.tgenabled WHEN 'D' THEN 'DISABLE' WHEN 'R' THEN 'ENABLE REPLICA' WHEN 'A' THEN 'ENABLE ALWAYS' END AS state,
       quote_ident(t.tgname) AS name
FROM pg_trigger t
WHERE t.tgrelid = $1 AND NOT t.tgisinternal AND t.tgparentid = 0
ORDER BY t.tgname COLLATE "C"
