select u.id, u.name, o.total, p.title
from users u inner join orders o on o.user_id = u.id
left outer join payments p on p.order_id = o.id and p.status = 'ok'
cross join settings s
natural join regions
right join x using (id)
where u.active
