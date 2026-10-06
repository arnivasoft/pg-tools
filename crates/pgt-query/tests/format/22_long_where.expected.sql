SELECT *
FROM orders
WHERE status = 'open'
  AND created_at BETWEEN '2026-01-01' AND '2026-12-31'
  AND (customer_id = 10 OR customer_id = 20)
  AND total > 1000
  AND region IN ('eu', 'us')
