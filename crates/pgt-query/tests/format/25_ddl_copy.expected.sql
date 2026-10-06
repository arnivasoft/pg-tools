create table t (
    id int primary key,
	name text not null
);

copy t from stdin with (format csv);

alter table t add column x int;
