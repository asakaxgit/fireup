# Multi-stage build for Fireup
FROM rust:1.75-slim as builder

# Install system dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    libpq-dev \
    && rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /app

# Copy dependency files
COPY Cargo.toml Cargo.lock ./

# Create a dummy main.rs to build dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build dependencies (this layer will be cached)
RUN cargo build --release && rm -rf src

# Copy source code
COPY src ./src

# Build the application
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    libpq5 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create app user
RUN useradd -r -s /bin/false fireup

# Create app directory
WORKDIR /app

# Copy binary from builder stage
COPY --from=builder /app/target/release/fireup /usr/local/bin/fireup

# Create directories for data
RUN mkdir -p /app/backups /app/output && \
    chown -R fireup:fireup /app

# Switch to app user
USER fireup

# Set default environment variables
ENV RUST_LOG=info
ENV FIREUP_MAX_BATCH_SIZE=1000
ENV FIREUP_CONNECTION_POOL_SIZE=10

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD fireup --version || exit 1

# Default command
ENTRYPOINT ["fireup"]
CMD ["--help"]