insert into counters (key, value) values ('hits', 1) on conflict (key) do update set value = counters.value + excluded.value where counters.key <> 'x';
insert into tags (name) values ('a') on conflict do nothing;
