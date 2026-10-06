create function f(a int) returns int language sql begin atomic select case when a > 0 then 1 else 2 end; select a; end;
select f(1);
