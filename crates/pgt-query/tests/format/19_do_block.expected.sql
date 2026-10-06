do $body$
declare   r record;
begin
  for r in select * from t loop
    raise notice '%', r;
  end loop;
end
$body$;
