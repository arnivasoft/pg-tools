UPDATE accounts a
SET balance = a.balance + t.amount, updated_at = now()
FROM transfers t
WHERE t.account_id = a.id AND t.applied = FALSE
RETURNING a.id, a.balance
