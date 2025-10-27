# Fireup Deployment Guide

This guide covers setting up Fireup for development and production environments.

## Development Environment Setup

### Prerequisites

- **Rust**: Version 1.70 or higher
- **Docker**: Version 20.10 or higher
- **Docker Compose**: Version 2.0 or higher
- **Git**: For cloning the repository

### Step-by-Step Setup

1. **Clone the Repository**
   ```bash
   git clone <repository-url>
   cd fireup
   ```

2. **Environment Configuration**
   ```bash
   # Copy the example environment file
   cp .env.example .env
   
   # Edit .env with your preferred settings
   nano .env
   ```

3. **Start PostgreSQL Backend**
   ```bash
   # Start PostgreSQL container
   docker-compose up -d postgres
   
   # Verify container is running
   docker-compose ps
   
   # Check logs for any issues
   docker-compose logs postgres
   ```

4. **Build the Application**
   ```bash
   # Build in development mode
   cargo build
   
   # Or build optimized release version
   cargo build --release
   ```

5. **Verify Installation**
   ```bash
   # Test CLI help
   cargo run -- --help
   
   # Test PostgreSQL connection
   psql -h localhost -p 5433 -U fireup -d fireup_dev -c "SELECT version();"
   ```

## Production Deployment

### Docker Compose Production Configuration

Create a `docker-compose.prod.yml` for production:

```yaml
version: '3.8'

services:
  postgres:
    image: postgres:15
    container_name: fireup_postgres_prod
    environment:
      POSTGRES_DB: ${POSTGRES_DB}
      POSTGRES_USER: ${POSTGRES_USER}
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD}
      POSTGRES_INITDB_ARGS: "--encoding=UTF8"
    ports:
      - "${POSTGRES_PORT}:5432"
    volumes:
      - postgres_prod_data:/var/lib/postgresql/data
      - ./init.sql:/docker-entrypoint-initdb.d/init.sql
    networks:
      - fireup_network
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U ${POSTGRES_USER} -d ${POSTGRES_DB}"]
      interval: 30s
      timeout: 10s
      retries: 3
      start_period: 60s
    restart: always
    deploy:
      resources:
        limits:
          memory: 2G
          cpus: '1.0'
        reservations:
          memory: 1G
          cpus: '0.5'

  fireup:
    build:
      context: .
      dockerfile: Dockerfile
    container_name: fireup_app
    environment:
      DATABASE_URL: postgresql://${POSTGRES_USER}:${POSTGRES_PASSWORD}@postgres:5432/${POSTGRES_DB}
      RUST_LOG: ${RUST_LOG:-info}
    volumes:
      - ./backups:/app/backups:ro
      - ./output:/app/output
    networks:
      - fireup_network
    depends_on:
      postgres:
        condition: service_healthy
    restart: unless-stopped

volumes:
  postgres_prod_data:
    driver: local

networks:
  fireup_network:
    driver: bridge
```

### Production Environment Variables

Create a `.env.prod` file:

```bash
# PostgreSQL Configuration
POSTGRES_HOST=postgres
POSTGRES_PORT=5432
POSTGRES_DB=fireup_production
POSTGRES_USER=fireup_prod
POSTGRES_PASSWORD=your_secure_password_here

# Application Configuration
FIREUP_MAX_BATCH_SIZE=5000
FIREUP_CONNECTION_POOL_SIZE=20
FIREUP_IMPORT_TIMEOUT_SECONDS=7200
RUST_LOG=fireup=info,warn
RUST_BACKTRACE=0
```

### Security Considerations

1. **Database Security**
   - Use strong passwords for PostgreSQL
   - Limit network access to PostgreSQL port
   - Enable SSL/TLS for database connections
   - Regular security updates for PostgreSQL image

2. **Application Security**
   - Run containers as non-root user
   - Use read-only file systems where possible
   - Implement proper backup file validation
   - Monitor for suspicious activity

3. **Network Security**
   - Use Docker networks for service isolation
   - Implement firewall rules
   - Consider using reverse proxy for external access

## PostgreSQL Client Connection Guide

### Command Line Tools

#### psql (PostgreSQL CLI)

```bash
# Development environment
psql -h localhost -p 5433 -U fireup -d fireup_dev

# Production environment
psql -h your-server -p 5432 -U fireup_prod -d fireup_production

# Common commands once connected:
\l                          # List databases
\dt fireup_data.*          # List tables in fireup_data schema
\d fireup_data.table_name  # Describe table structure
\q                         # Quit
```

#### pg_dump (Backup)

```bash
# Backup imported data
pg_dump -h localhost -p 5433 -U fireup -d fireup_dev \
  --schema=fireup_data --data-only > fireup_backup.sql

# Backup schema only
pg_dump -h localhost -p 5433 -U fireup -d fireup_dev \
  --schema=fireup_data --schema-only > fireup_schema.sql
```

### GUI Tools

#### pgAdmin

1. **Installation**: Download from https://www.pgadmin.org/
2. **Connection Setup**:
   - Host: localhost (or your server IP)
   - Port: 5433 (development) or 5432 (production)
   - Database: fireup_dev or fireup_production
   - Username: fireup or fireup_prod
   - Password: (from your .env file)

#### DBeaver

1. **Installation**: Download from https://dbeaver.io/
2. **Connection Setup**:
   - Database: PostgreSQL
   - Server Host: localhost
   - Port: 5433
   - Database: fireup_dev
   - Username: fireup
   - Password: fireup_dev_password

### Programming Language Connections

#### Python (psycopg2)

```python
import psycopg2
from psycopg2.extras import RealDictCursor

# Connection
conn = psycopg2.connect(
    host="localhost",
    port=5433,
    database="fireup_dev",
    user="fireup",
    password="fireup_dev_password"
)

# Query with cursor
with conn.cursor(cursor_factory=RealDictCursor) as cur:
    cur.execute("SELECT * FROM fireup_data.import_summary")
    results = cur.fetchall()
    for row in results:
        print(row)

conn.close()
```

#### Node.js (pg)

```javascript
const { Client } = require('pg');

const client = new Client({
  host: 'localhost',
  port: 5433,
  database: 'fireup_dev',
  user: 'fireup',
  password: 'fireup_dev_password',
});

async function queryData() {
  await client.connect();
  
  const res = await client.query('SELECT * FROM fireup_data.import_summary');
  console.log(res.rows);
  
  await client.end();
}

queryData().catch(console.error);
```

## Monitoring and Maintenance

### Health Checks

```bash
# Check PostgreSQL health
docker-compose exec postgres pg_isready -U fireup -d fireup_dev

# Check application logs
docker-compose logs -f fireup

# Monitor resource usage
docker stats
```

### Database Maintenance

```sql
-- Check database size
SELECT pg_size_pretty(pg_database_size('fireup_dev'));

-- Check table sizes
SELECT 
    schemaname,
    tablename,
    pg_size_pretty(pg_total_relation_size(schemaname||'.'||tablename)) as size
FROM pg_tables 
WHERE schemaname = 'fireup_data'
ORDER BY pg_total_relation_size(schemaname||'.'||tablename) DESC;

-- Vacuum and analyze tables
VACUUM ANALYZE;
```

### Backup and Recovery

```bash
# Create full backup
docker-compose exec postgres pg_dump -U fireup fireup_dev > backup_$(date +%Y%m%d).sql

# Restore from backup
docker-compose exec -T postgres psql -U fireup fireup_dev < backup_20231201.sql

# Automated backup script
#!/bin/bash
BACKUP_DIR="/backups"
DATE=$(date +%Y%m%d_%H%M%S)
docker-compose exec postgres pg_dump -U fireup fireup_dev > "$BACKUP_DIR/fireup_$DATE.sql"
find "$BACKUP_DIR" -name "fireup_*.sql" -mtime +7 -delete
```

## Troubleshooting

### Common Issues and Solutions

1. **Container Won't Start**
   ```bash
   # Check Docker daemon
   docker info
   
   # Check port conflicts
   netstat -tulpn | grep :5433
   
   # View detailed logs
   docker-compose logs postgres
   ```

2. **Connection Refused**
   ```bash
   # Verify container is running
   docker-compose ps
   
   # Check network connectivity
   docker-compose exec postgres netstat -tlnp
   
   # Test connection from host
   telnet localhost 5433
   ```

3. **Permission Denied**
   ```bash
   # Check PostgreSQL logs
   docker-compose logs postgres | grep ERROR
   
   # Verify user permissions
   docker-compose exec postgres psql -U fireup -d fireup_dev -c "\du"
   ```

4. **Out of Memory**
   ```bash
   # Check container memory usage
   docker stats
   
   # Increase memory limits in docker-compose.yml
   # Reduce batch size in application configuration
   ```

5. **Slow Performance**
   ```sql
   -- Check for missing indexes
   SELECT * FROM pg_stat_user_tables WHERE schemaname = 'fireup_data';
   
   -- Analyze query performance
   EXPLAIN ANALYZE SELECT * FROM fireup_data.your_table;
   
   -- Update table statistics
   ANALYZE;
   ```

### Log Analysis

```bash
# Application logs
docker-compose logs fireup | grep ERROR

# PostgreSQL logs
docker-compose logs postgres | grep -E "(ERROR|FATAL|PANIC)"

# Real-time monitoring
docker-compose logs -f --tail=100
```

### Performance Tuning

1. **PostgreSQL Configuration**
   - Adjust `shared_buffers` for available memory
   - Tune `work_mem` for complex queries
   - Configure `checkpoint_segments` for write-heavy workloads

2. **Application Configuration**
   - Increase `FIREUP_MAX_BATCH_SIZE` for faster imports
   - Adjust `FIREUP_CONNECTION_POOL_SIZE` based on concurrent load
   - Monitor memory usage and adjust accordingly

3. **System Resources**
   - Ensure adequate disk space for data and logs
   - Monitor CPU usage during large imports
   - Consider SSD storage for better I/O performance