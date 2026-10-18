-- CLUSTER / VACUUM FULL ilerlemesi (12+). Sıralı tarama blok, indeks taraması/yazma satır, indeks yeniden kurma
-- indeks sayısıyla; diğer aşamalar belirsiz. Yalnız uygulama bağlantısının pid'i ($1).
SELECT coalesce(p.phase::text, 'unknown') AS phase,
       (CASE WHEN p.phase = 'seq scanning heap' THEN p.heap_blks_scanned
             WHEN p.phase IN ('index scanning heap', 'writing new heap') THEN p.heap_tuples_written
             WHEN p.phase = 'rebuilding index' THEN p.index_rebuild_count END)::int8 AS done,
       (CASE WHEN p.phase = 'seq scanning heap' THEN p.heap_blks_total END)::int8 AS total,
       CASE WHEN p.phase = 'seq scanning heap' THEN 'blocks'
            WHEN p.phase IN ('index scanning heap', 'writing new heap') THEN 'tuples'
            WHEN p.phase = 'rebuilding index' THEN 'indexes' END::text AS unit
  FROM pg_stat_progress_cluster p
 WHERE p.pid = $1
