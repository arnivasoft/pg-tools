SELECT
  payload->>'name' AS name,
  (payload->'meta')::jsonb #>> '{a,b}',
  created::date,
  x :: int,
  '{"a":1}'::json -> 'a',
  arr[1],
  ts at time zone 'UTC'
FROM events
WHERE payload ? 'name' AND tags @> ARRAY['x']
