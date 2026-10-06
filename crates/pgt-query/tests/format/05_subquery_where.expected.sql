SELECT *
FROM users
WHERE id IN (
  SELECT user_id
  FROM orders
  WHERE total > 100
)
  AND NOT EXISTS (
    SELECT 1
    FROM bans b
    WHERE b.user_id = users.id
  )
