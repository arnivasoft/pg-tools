-- Composite (ya da satır tipi) öznitelikleri, attnum sırasıyla, silinmişler hariç. $1 = pg_type oid'i.
-- type_name: format_type (değiştiricili, görünmeyen şema nitelikli). collation yalnızca tipin varsayılanından FARKLIysa
-- (şema niteliksiz ad).
SELECT a.attname::text AS name,
       format_type(a.atttypid, a.atttypmod) AS type_name,
       CASE WHEN a.attcollation <> at.typcollation THEN co.collname::text END AS collation
FROM pg_type t
JOIN pg_attribute a ON a.attrelid = t.typrelid
JOIN pg_type at ON at.oid = a.atttypid
LEFT JOIN pg_collation co ON co.oid = a.attcollation
WHERE t.oid = $1 AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY a.attnum
