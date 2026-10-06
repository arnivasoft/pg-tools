select * from orders where status = 'open' and created_at between '2026-01-01' and '2026-12-31' and (customer_id = 10 or customer_id = 20) and total > 1000 and region in ('eu', 'us')
