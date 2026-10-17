-- Enum etiketleri, enumsortorder sırasıyla. $1 = pg_type oid'i.
SELECT e.enumlabel::text AS label
FROM pg_enum e
WHERE e.enumtypid = $1
ORDER BY e.enumsortorder
