-- Additional development setup for Fireup
-- This script runs after the main init.sql

-- Create sample tables for testing schema generation
CREATE TABLE IF NOT EXISTS fireup_data.sample_users (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    email VARCHAR(255) UNIQUE NOT NULL,
    name VARCHAR(100) NOT NULL,
    created_at TIMESTAMP DEFAULT NOW(),
    metadata JSONB
);

-- Create sample data for testing
INSERT INTO fireup_data.sample_users (email, name, metadata) VALUES
('test@example.com', 'Test User', '{"role": "admin", "preferences": {"theme": "dark"}}'),
('demo@example.com', 'Demo User', '{"role": "user", "preferences": {"theme": "light"}}')
ON CONFLICT (email) DO NOTHING;

-- Create a view for monitoring import progress
CREATE OR REPLACE VIEW fireup_data.import_summary AS
SELECT 
    operation_type,
    status,
    COUNT(*) as operation_count,
    SUM(records_processed) as total_records,
    SUM(errors_count) as total_errors,
    MIN(started_at) as first_operation,
    MAX(completed_at) as last_completed
FROM fireup_data.import_audit
GROUP BY operation_type, status
ORDER BY operation_type, status;

-- Grant view permissions
GRANT SELECT ON fireup_data.import_summary TO fireup;