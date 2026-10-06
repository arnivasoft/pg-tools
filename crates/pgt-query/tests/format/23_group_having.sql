select dept, count(*), avg(salary) from emp group by dept, grade having count(*) > 5 order by 2 desc, 1 asc limit all offset 10 fetch first 5 rows only
