-- Fonksiyon dilleri: prosedürel diller (lanispl) ve `sql`; `c`/`internal` hariç (Karar 4: salt okunur).
-- detail: güvenilirlik (güvenilmez dilde fonksiyonu yalnızca süper kullanıcı oluşturabilir).
SELECT l.lanname::text AS value,
       CASE WHEN l.lanpltrusted THEN 'güvenilir' ELSE 'güvenilmez' END AS detail
FROM pg_language l
WHERE (l.lanispl OR l.lanname = 'sql')
  AND l.lanname NOT IN ('c', 'internal')
ORDER BY l.lanname
