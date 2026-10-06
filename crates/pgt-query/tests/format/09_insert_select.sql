insert into archive (id, name, created_at) select id, name, created_at from users where deleted returning id;
