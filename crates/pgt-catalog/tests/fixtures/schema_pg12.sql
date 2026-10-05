CREATE TABLE extra.line_items (
    qty integer NOT NULL,
    price numeric(10, 2) NOT NULL,
    total numeric(12, 2) GENERATED ALWAYS AS (qty * price) STORED
);
