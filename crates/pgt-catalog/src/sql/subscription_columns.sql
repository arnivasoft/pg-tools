-- pg_subscription'ın bu sunucuda var olan ve oturum rolünce okunabilen sütunları (subconninfo yalnız yetkiliye açıktır;
-- sürüme göre sütun kümesi değişir).
SELECT a.attname::text AS col
FROM pg_attribute a
WHERE a.attrelid = 'pg_catalog.pg_subscription'::regclass AND a.attnum > 0 AND NOT a.attisdropped
  AND has_column_privilege('pg_catalog.pg_subscription'::regclass, a.attnum, 'SELECT')
