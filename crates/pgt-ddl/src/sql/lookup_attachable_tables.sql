-- ATTACH PARTITION için bağlanabilir tablolar. $1 = üst tablonun oid'i (metin olarak gelir). Hariç: üst tablonun kendisi
-- ve ataları (alt bölümlemede), bölümler, kalıtım üyeleri/üstleri (pg_inherits'te çocuk ya da üst olanlar), sistem
-- şemaları ve eklentiye ait tablolar. Bölümlü ama bölümsüz tablo listelenir. value = tablo adı, detail = şema; en çok 5000.
-- anc CTE'si derinlemesine savunmadır; NOT EXISTS (inhparent) koşulu zaten üst tabloyu ve atalarını kapsar.
WITH RECURSIVE anc(oid) AS (
    SELECT $1::text::oid
    UNION
    SELECT i.inhparent FROM pg_inherits i JOIN anc a ON i.inhrelid = a.oid
)
SELECT c.relname::text AS value, n.nspname::text AS detail
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.relkind IN ('r', 'p')
  AND NOT c.relispartition
  AND c.oid NOT IN (SELECT oid FROM anc)
  AND NOT EXISTS (SELECT 1 FROM pg_inherits i WHERE i.inhrelid = c.oid OR i.inhparent = c.oid)
  AND left(n.nspname, 3) <> 'pg_' AND n.nspname <> 'information_schema'
  AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid
                    AND d.refclassid = 'pg_extension'::regclass AND d.deptype = 'e')
ORDER BY n.nspname::text COLLATE "C", c.relname::text COLLATE "C"
LIMIT 5000
