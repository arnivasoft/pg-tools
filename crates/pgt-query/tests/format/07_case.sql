select id, case when status = 1 then 'active' when status = 2 then 'blocked' else 'unknown' end as status_text, case kind when 'a' then 1 else 0 end from users
