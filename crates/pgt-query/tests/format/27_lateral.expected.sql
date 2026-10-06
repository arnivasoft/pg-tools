SELECT u.id, l.*
FROM users u
CROSS JOIN LATERAL (
  SELECT *
  FROM logins
  WHERE user_id = u.id
  ORDER BY at DESC
  LIMIT 3
) l
