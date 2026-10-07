-- ALTER DATABASE|ROLE … SET ile ayarlanabilen parametreler (context user/superuser), ad küçük harfle (üretici yalnızca
-- küçük harf ad kabul eder; GUC adları harfe duyarsızdır). detail: tip · kısa açıklama.
SELECT lower(name) AS value,
       concat_ws(' · ', vartype, short_desc) AS detail
FROM pg_settings
WHERE context IN ('user', 'superuser')
ORDER BY lower(name) COLLATE "C"
