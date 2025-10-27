-- Fireup PostgreSQL initialization script
-- This script sets up the initial database structure for development

-- Enable UUID extension for generating UUIDs
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- Create schema for migrated data
CREATE SCHEMA IF NOT EXISTS fireup_data;

-- Set default search path
ALTER DATABASE fireup_dev SET search_path TO fireup_data, public;

-- Create audit table for tracking import operations
CREATE TABLE IF NOT EXISTS fireup_data.import_audit (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    operation_type VARCHAR(50) NOT NULL,
    backup_file_path TEXT,
    started_at TIMESTAMP NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMP,
    status VARCHAR(20) NOT NULL DEFAULT 'in_progress',
    records_processed INTEGER DEFAULT 0,
    errors_count INTEGER DEFAULT 0,
    metadata JSONB,
    created_by VARCHAR(100) DEFAULT 'fireup_system'
);

-- Create index on audit table for performance
CREATE INDEX IF NOT EXISTS idx_import_audit_started_at ON fireup_data.import_audit(started_at);
CREATE INDEX IF NOT EXISTS idx_import_audit_status ON fireup_data.import_audit(status);

-- Grant permissions to fireup user
GRANT ALL PRIVILEGES ON SCHEMA fireup_data TO fireup;
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA fireup_data TO fireup;
GRANT ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA fireup_data TO fireup;

-- Set default privileges for future objects
ALTER DEFAULT PRIVILEGES IN SCHEMA fireup_data GRANT ALL ON TABLES TO fireup;
ALTER DEFAULT PRIVILEGES IN SCHEMA fireup_data GRANT ALL ON SEQUENCES TO fireup;

-- Log successful initialization
INSERT INTO fireup_data.import_audit (operation_type, status, completed_at, metadata)
VALUES ('database_init', 'completed', NOW(), '{"message": "Database initialized successfully"}');