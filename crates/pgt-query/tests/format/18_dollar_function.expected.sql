create or replace function add(a int, b int) returns int
language plpgsql as $$
begin
    return a + b;   -- toplama
end;
$$;

SELECT add(1, 2);
