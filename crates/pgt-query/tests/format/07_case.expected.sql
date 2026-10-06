SELECT
  id,
  CASE WHEN status = 1 THEN 'active' WHEN status = 2 THEN 'blocked' ELSE 'unknown' END AS status_text,
  CASE kind WHEN 'a' THEN 1 ELSE 0 END
FROM users
