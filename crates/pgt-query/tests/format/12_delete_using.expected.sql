DELETE FROM orders o
USING users u
WHERE o.user_id = u.id AND u.deleted
RETURNING o.id
