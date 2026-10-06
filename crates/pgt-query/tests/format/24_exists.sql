select * from a where exists(select 1 from b where b.a_id = a.id) or a.id = any(select id from c)
