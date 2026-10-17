-- Domain kısıtları (tüm sürümler). $1 = pg_type oid'i. contype 'c' CHECK; PG 17+ 'n' NOT NULL satırları Rust'ta
-- ayrılır (typnotnull ile modellenir). def: pg_get_constraintdef(oid, true) → "CHECK (<ifade>)[ NOT VALID]".
SELECT c.conname::text AS name,
       c.contype::text AS contype,
       c.convalidated AS validated,
       pg_get_constraintdef(c.oid, true) AS def
FROM pg_constraint c
WHERE c.contypid = $1
