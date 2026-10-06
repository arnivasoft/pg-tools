SELECT dept, count(*), avg(salary)
FROM emp
GROUP BY dept, grade
HAVING count(*) > 5
ORDER BY 2 DESC, 1 ASC
LIMIT ALL
OFFSET 10
FETCH FIRST 5 ROWS ONLY
