SELECT stmt FROM (
  SELECT format('GRANT %s ON TABLE %s TO %s%s;',
                string_agg(a2.privilege_type || ' (' || quote_ident(a.attname) || ')', ', ' ORDER BY a.attnum, a2.privilege_type),
                quote_ident(n.nspname) || '.' || quote_ident(c.relname),
                CASE WHEN a2.grantee = 0 THEN 'PUBLIC' ELSE quote_ident(pg_get_userbyid(a2.grantee)) END,
                CASE WHEN a2.is_grantable THEN ' WITH GRANT OPTION' ELSE '' END) AS stmt
  FROM pg_attribute a
  JOIN pg_class c ON c.oid = a.attrelid
  JOIN pg_namespace n ON n.oid = c.relnamespace,
  LATERAL aclexplode(a.attacl) a2
  WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped AND a.attacl IS NOT NULL
  GROUP BY n.nspname, c.relname, a2.grantee, a2.is_grantable
) x
ORDER BY stmt COLLATE "C"
