select s.user_id, s.n from (select user_id, count(*) as n from orders group by user_id) as s where s.n > (select avg(c) from stats)
