-- Kolon seçenekleri (attfdwoptions); `name` table_ddl_columns.sql ile aynı biçimde (quote_ident) eşleşme anahtarıdır.
SELECT quote_ident(a.attname) AS name,
       COALESCE(a.attfdwoptions, '{}') AS options,
       ARRAY(SELECT quote_ident(o.k) FROM pg_options_to_table(a.attfdwoptions) WITH ORDINALITY o(k, v, n) ORDER BY o.n) AS option_keys
FROM pg_attribute a
WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped AND a.attfdwoptions IS NOT NULL
ORDER BY a.attnum
