-- REINDEX ilerlemesi (12+; pg_stat_progress_create_index). Blok sayacı varsa blok, yoksa satır; ikisi de yoksa
-- belirsiz. Yalnız uygulama bağlantısının pid'i ($1).
SELECT coalesce(p.phase::text, 'unknown') AS phase,
       (CASE WHEN p.blocks_total > 0 THEN p.blocks_done WHEN p.tuples_total > 0 THEN p.tuples_done END)::int8 AS done,
       (CASE WHEN p.blocks_total > 0 THEN p.blocks_total WHEN p.tuples_total > 0 THEN p.tuples_total END)::int8 AS total,
       CASE WHEN p.blocks_total > 0 THEN 'blocks' WHEN p.tuples_total > 0 THEN 'tuples' END::text AS unit
  FROM pg_stat_progress_create_index p
 WHERE p.pid = $1
