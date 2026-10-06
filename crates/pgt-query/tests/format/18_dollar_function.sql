create or replace function add(a int,   b int) returns int
language plpgsql as $$
begin
    return a + b;   -- toplama
end;
$$;
select add(1, 2);
