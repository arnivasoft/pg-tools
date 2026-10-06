SELECT CASE
  WHEN a > 0 THEN CASE WHEN b > 0 THEN 'both positive values found here' ELSE 'only a is positive' END
  ELSE 'a is not positive at all here'
END AS label
FROM t
