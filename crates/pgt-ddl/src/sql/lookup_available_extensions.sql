-- Sunucuda kurulabilir eklentiler.
SELECT name::text AS value,
       default_version::text AS default_version,
       installed_version::text AS installed_version,
       comment::text AS comment
FROM pg_available_extensions
ORDER BY name
