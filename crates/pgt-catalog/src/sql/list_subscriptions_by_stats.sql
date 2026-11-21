-- pg_subscription.oid herkese açık değilse (PG 10–13 sütun yetkisi) yedek liste: oid, herkese açık
-- pg_stat_subscription görünümünden ada göre alınır. Ad yalnız veritabanı içinde benzersizdir; başka veritabanında
-- aynı adlı abonelik varsa eşleme belirsizdir ve abonelik listelenmez.
SELECT st.subid AS oid, s.subname::text AS name, CASE WHEN s.subenabled THEN 'etkin' ELSE 'devre dışı' END AS detail, true AS leaf
FROM pg_subscription s
JOIN (SELECT x.subname, min(x.subid) AS subid FROM pg_stat_subscription x GROUP BY x.subname HAVING count(DISTINCT x.subid) = 1) st
  ON st.subname = s.subname
WHERE s.subdbid = (SELECT d.oid FROM pg_database d WHERE d.datname = current_database())
ORDER BY s.subname::text COLLATE "C"
