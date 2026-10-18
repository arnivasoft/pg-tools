-- ANALYZE ilerlemesi (13+). Örnek blok sayacı; yoksa belirsiz. Yalnız uygulama bağlantısının pid'i ($1).
SELECT coalesce(p.phase::text, 'unknown') AS phase,
       (CASE WHEN p.sample_blks_total > 0 THEN p.sample_blks_scanned END)::int8 AS done,
       (CASE WHEN p.sample_blks_total > 0 THEN p.sample_blks_total END)::int8 AS total,
       CASE WHEN p.sample_blks_total > 0 THEN 'blocks' END::text AS unit
  FROM pg_stat_progress_analyze p
 WHERE p.pid = $1
