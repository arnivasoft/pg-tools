select * from users where id in (select user_id from orders where total > 100) and not exists (select 1 from bans b where b.user_id = users.id)
