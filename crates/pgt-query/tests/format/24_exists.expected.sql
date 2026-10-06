SELECT *
FROM a
WHERE EXISTS(
  SELECT 1
  FROM b
  WHERE b.a_id = a.id
)
  OR a.id = ANY(
    SELECT id
    FROM c
  )
