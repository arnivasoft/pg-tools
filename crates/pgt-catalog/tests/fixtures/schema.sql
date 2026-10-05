DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pgt_reader') THEN
        CREATE ROLE pgt_reader NOLOGIN;
    END IF;
END
$$;

CREATE SCHEMA app;
COMMENT ON SCHEMA app IS 'Uygulama şeması';
GRANT USAGE ON SCHEMA app TO pgt_reader;

CREATE TABLE app.customers (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    email text NOT NULL UNIQUE,
    name varchar(100) COLLATE "C",
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT customers_email_check CHECK (position('@' in email) > 1)
);
COMMENT ON TABLE app.customers IS 'Müşteriler';
COMMENT ON COLUMN app.customers.email IS 'E-posta';
GRANT SELECT, INSERT ON app.customers TO pgt_reader;

CREATE TABLE app.orders (
    id bigserial PRIMARY KEY,
    customer_id bigint NOT NULL REFERENCES app.customers (id) ON DELETE CASCADE,
    total numeric(12, 2) NOT NULL DEFAULT 0,
    placed_on date NOT NULL
) WITH (fillfactor = 80);

CREATE TABLE app.events (
    id bigint NOT NULL,
    happened_on date NOT NULL,
    payload jsonb
) PARTITION BY RANGE (happened_on);
CREATE TABLE app.events_2026 PARTITION OF app.events FOR VALUES FROM ('2026-01-01') TO ('2027-01-01');

CREATE TABLE app."Order Items" (
    "Line No" integer NOT NULL,
    order_id bigint NOT NULL REFERENCES app.orders (id),
    "Ürün Adı" text,
    PRIMARY KEY (order_id, "Line No")
);

CREATE UNLOGGED TABLE public.scratch (v text);
