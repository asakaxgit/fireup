-- Fireup PostgreSQL initialization script
-- This script sets up the initial database structure for development.
--
-- It is mounted at /docker-entrypoint-initdb.d/init.sql by docker-compose.yml
-- and runs once, when the postgres data volume is first created.
--
-- Scope: create the schema the importer writes into, and grant the importer's
-- role access to it. Nothing here describes or records an import; the importer
-- creates its own tables from the analysed Firestore schema at import time.

-- Create schema for migrated data
CREATE SCHEMA IF NOT EXISTS fireup_data;

-- Set default search path
ALTER DATABASE fireup_dev SET search_path TO fireup_data, public;

-- Grant permissions to fireup user
GRANT ALL PRIVILEGES ON SCHEMA fireup_data TO fireup;
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA fireup_data TO fireup;
GRANT ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA fireup_data TO fireup;

-- Set default privileges for future objects, so the tables the importer
-- creates after this script runs are reachable by the fireup role too.
ALTER DEFAULT PRIVILEGES IN SCHEMA fireup_data GRANT ALL ON TABLES TO fireup;
ALTER DEFAULT PRIVILEGES IN SCHEMA fireup_data GRANT ALL ON SEQUENCES TO fireup;
