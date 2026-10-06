SELECT name::text, setting::text AS value, unit::text, category::text, short_desc::text AS description,
       source::text, context::text, boot_val::text AS boot_value, pending_restart
FROM pg_catalog.pg_settings
ORDER BY name
