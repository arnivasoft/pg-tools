update accounts a set balance = a.balance + t.amount, updated_at = now() from transfers t where t.account_id = a.id and t.applied = false returning a.id, a.balance
