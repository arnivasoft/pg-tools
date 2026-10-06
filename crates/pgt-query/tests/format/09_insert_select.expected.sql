INSERT INTO archive (id, name, created_at)
SELECT id, name, created_at
FROM users
WHERE deleted
RETURNING id;
