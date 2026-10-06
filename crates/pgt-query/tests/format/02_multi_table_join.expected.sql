SELECT u.id, u.name, o.total, p.title
FROM users u
INNER JOIN orders o ON o.user_id = u.id
LEFT OUTER JOIN payments p ON p.order_id = o.id AND p.status = 'ok'
CROSS JOIN settings s
NATURAL JOIN regions
RIGHT JOIN x USING (id)
WHERE u.active
