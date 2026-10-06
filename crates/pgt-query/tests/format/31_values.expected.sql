VALUES (1, 'a'), (2, 'b');

SELECT *
FROM (
  VALUES (1, 'x'), (2, 'y')
) AS v(id, name);
