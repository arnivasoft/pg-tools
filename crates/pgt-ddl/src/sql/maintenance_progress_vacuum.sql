-- VACUUM ilerlemesi (9.6+; FULL'suz VACUUM). Sayaç aşamaya göre: `scanning heap` taranan, `vacuuming heap`
-- vakumlanan blok (toplam `heap_blks_total`); diğer aşamalar (indeks vakumu, temizlik, kesme) belirsiz. Sayaç ya da
-- toplam boşsa üçü de boş. Yalnız uygulama bağlantısının pid'i ($1).
SELECT q.phase,
       CASE WHEN q.total IS NOT NULL THEN q.done END AS done,
       CASE WHEN q.done IS NOT NULL THEN q.total END AS total,
       CASE WHEN q.done IS NOT NULL AND q.total IS NOT NULL THEN 'blocks' END::text AS unit
  FROM (SELECT coalesce(p.phase::text, 'unknown') AS phase,
               (CASE WHEN p.phase = 'scanning heap' THEN p.heap_blks_scanned
                     WHEN p.phase = 'vacuuming heap' THEN p.heap_blks_vacuumed END)::int8 AS done,
               (CASE WHEN p.phase IN ('scanning heap', 'vacuuming heap') THEN p.heap_blks_total END)::int8 AS total
          FROM pg_stat_progress_vacuum p
         WHERE p.pid = $1) q
