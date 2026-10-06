select id, row_number() over (partition by dept order by salary desc) as rn, sum(salary) over w, count(*) filter (where active) from emp window w as (partition by dept) order by dept nulls first
