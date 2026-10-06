select distinct on (user_id) user_id, created_at from events order by user_id, created_at desc
