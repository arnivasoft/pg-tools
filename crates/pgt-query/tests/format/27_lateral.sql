select u.id, l.* from users u cross join lateral (select * from logins where user_id = u.id order by at desc limit 3) l
