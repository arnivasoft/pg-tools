WITH recent AS (
  SELECT *
  FROM orders
  WHERE created_at > now() - interval '7 days'
),
totals AS MATERIALIZED (
  SELECT user_id, sum(total) AS s
  FROM recent
  GROUP BY user_id
)
SELECT u.name, t.s
FROM users u
JOIN totals t ON t.user_id = u.id
ORDER BY t.s DESC;
