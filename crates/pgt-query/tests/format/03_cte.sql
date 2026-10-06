with recent as (select * from orders where created_at > now() - interval '7 days'), totals as materialized (select user_id, sum(total) as s from recent group by user_id)
select u.name, t.s from users u join totals t on t.user_id = u.id order by t.s desc;
