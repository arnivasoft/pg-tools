INSERT INTO counters (key, value)
VALUES ('hits', 1)
ON CONFLICT (key) DO UPDATE
SET value = counters.value + excluded.value
WHERE counters.key <> 'x';

INSERT INTO tags (name)
VALUES ('a')
ON CONFLICT DO NOTHING;
