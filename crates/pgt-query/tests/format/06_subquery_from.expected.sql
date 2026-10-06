SELECT s.user_id, s.n
FROM (
  SELECT user_id, count(*) AS n
  FROM orders
  GROUP BY user_id
) AS s
WHERE s.n > (
  SELECT avg(c)
  FROM stats
)
