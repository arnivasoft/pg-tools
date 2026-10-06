SELECT
  id,
  row_number() OVER (PARTITION BY dept ORDER BY salary DESC) AS rn,
  sum(salary) OVER w,
  count(*) FILTER (WHERE active)
FROM emp
WINDOW w AS (PARTITION BY dept)
ORDER BY dept NULLS FIRST
