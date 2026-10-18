-- REINDEX ilerlemesi (12+; pg_stat_progress_create_index). Sayaç aşamaya göre: tarama aşamaları (`... scanning
-- table`, `index validation: scanning index/table`) blok, ağaca yükleme (`building index: loading tuples in tree`)
-- satır — önceki aşamanın blok sayacı dolu kalabildiği için aşamaya bakılır. Sayacın toplamı yoksa (0) ya da başka
-- aşamaysa belirsiz. Yalnız uygulama bağlantısının pid'i ($1).
SELECT coalesce(p.phase::text, 'unknown') AS phase,
       (CASE WHEN p.phase LIKE '%loading tuples in tree' AND p.tuples_total > 0 THEN p.tuples_done
             WHEN p.phase LIKE '%scanning%' AND p.blocks_total > 0 THEN p.blocks_done END)::int8 AS done,
       (CASE WHEN p.phase LIKE '%loading tuples in tree' AND p.tuples_total > 0 THEN p.tuples_total
             WHEN p.phase LIKE '%scanning%' AND p.blocks_total > 0 THEN p.blocks_total END)::int8 AS total,
       CASE WHEN p.phase LIKE '%loading tuples in tree' AND p.tuples_total > 0 THEN 'tuples'
            WHEN p.phase LIKE '%scanning%' AND p.blocks_total > 0 THEN 'blocks' END::text AS unit
  FROM pg_stat_progress_create_index p
 WHERE p.pid = $1
