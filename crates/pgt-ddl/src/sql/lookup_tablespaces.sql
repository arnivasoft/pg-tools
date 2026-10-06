-- İndekse atanabilen tablespace'ler (pg_default dahil, pg_global hariç).
SELECT spcname::text AS value, NULL::text AS detail
FROM pg_tablespace
WHERE spcname <> 'pg_global'
ORDER BY (spcname = 'pg_default') DESC, spcname
