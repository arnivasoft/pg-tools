select * from jobs where state = 'queued' order by id limit 1 for update skip locked
