delete from orders o using users u where o.user_id = u.id and u.deleted returning o.id
