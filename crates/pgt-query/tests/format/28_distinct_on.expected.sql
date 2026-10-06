SELECT DISTINCT ON (user_id) user_id, created_at
FROM events
ORDER BY user_id, created_at DESC
