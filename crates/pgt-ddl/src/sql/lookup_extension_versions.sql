-- Bir eklentinin kurulabilir sürümleri (UPDATE TO seçicisi). $1 = eklenti adı. detail: yüklü bayrağı + açıklama.
SELECT version::text AS value,
       NULLIF(concat_ws(' · ', CASE WHEN installed THEN 'yüklü' END, comment), '') AS detail
FROM pg_available_extension_versions
WHERE name = $1::text
ORDER BY version COLLATE "C" DESC
