-- Sunucudaki indeks erişim yöntemleri (kesişim Rust tarafında sabit altı yöntemle yapılır).
SELECT amname::text AS value, NULL::text AS detail
FROM pg_am
WHERE amtype = 'i'
