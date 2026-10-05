CREATE PROCEDURE app.archive_orders(older_than date) LANGUAGE sql AS $$ DELETE FROM app.orders WHERE placed_on < older_than $$;
COMMENT ON PROCEDURE app.archive_orders(date) IS 'Eski siparişleri siler';
CREATE INDEX events_day_idx ON app.events (happened_on);
CREATE INDEX orders_cover_idx ON app.orders (customer_id) INCLUDE (total);
CREATE TABLE extra.hits (id bigint, bucket int) PARTITION BY HASH (id);
CREATE TABLE extra.hits_0 PARTITION OF extra.hits FOR VALUES WITH (MODULUS 2, REMAINDER 0);
CREATE TABLE extra.hits_1 PARTITION OF extra.hits FOR VALUES WITH (MODULUS 2, REMAINDER 1);
