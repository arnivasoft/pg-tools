-- VACUUM ilerlemesi (9.6+; FULL'suz VACUUM). Yalnız uygulama bağlantısının pid'i ($1).
SELECT coalesce(p.phase::text, 'unknown') AS phase,
       p.heap_blks_scanned::int8 AS done,
       p.heap_blks_total::int8 AS total,
       'blocks'::text AS unit
  FROM pg_stat_progress_vacuum p
 WHERE p.pid = $1
