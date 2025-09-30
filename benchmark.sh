#!/bin/bash
# RustStrom Benchmarking Script
# Compare RustStrom performance against HAProxy and other load balancers

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Configuration
DURATION="60s"
CONNECTIONS="1000"
THREADS="8"
BACKEND_SERVERS=3

print_banner() {
    echo -e "${BLUE}"
    cat << "EOF"
╔═══════════════════════════════════════════════════════╗
║         RustStrom Benchmark Suite                     ║
║         Performance Testing Tool                      ║
╚═══════════════════════════════════════════════════════╝
EOF
    echo -e "${NC}"
}

print_info() {
    echo -e "${BLUE}ℹ${NC} $1"
}

print_success() {
    echo -e "${GREEN}✓${NC} $1"
}

print_error() {
    echo -e "${RED}✗${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}⚠${NC} $1"
}

# Check if wrk is installed
check_wrk() {
    if ! command -v wrk &> /dev/null; then
        print_error "wrk is not installed"
        echo "Install with:"
        echo "  Ubuntu/Debian: sudo apt install wrk"
        echo "  macOS: brew install wrk"
        echo "  From source: git clone https://github.com/wg/wrk.git && cd wrk && make"
        exit 1
    fi
    print_success "wrk found: $(wrk --version 2>&1 | head -1)"
}

# Start backend servers
start_backends() {
    print_info "Starting backend servers..."
    
    for i in $(seq 1 $BACKEND_SERVERS); do
        PORT=$((8080 + i - 1))
        mkdir -p /tmp/backend_$PORT
        echo "<html><body><h1>Backend Server $PORT</h1><p>Request served by port $PORT</p></body></html>" > /tmp/backend_$PORT/index.html
        
        # Start simple HTTP server
        cd /tmp/backend_$PORT
        python3 -m http.server $PORT > /dev/null 2>&1 &
        BACKEND_PID=$!
        echo $BACKEND_PID > /tmp/backend_$PORT.pid
        print_success "Backend server started on port $PORT (PID: $BACKEND_PID)"
    done
    
    sleep 2
    print_success "All backend servers started"
}

# Stop backend servers
stop_backends() {
    print_info "Stopping backend servers..."
    
    for i in $(seq 1 $BACKEND_SERVERS); do
        PORT=$((8080 + i - 1))
        if [ -f /tmp/backend_$PORT.pid ]; then
            PID=$(cat /tmp/backend_$PORT.pid)
            kill $PID 2>/dev/null || true
            rm /tmp/backend_$PORT.pid
            print_success "Backend server on port $PORT stopped"
        fi
    done
}

# Create test config for RustStrom
create_config() {
    print_info "Creating test configuration..."
    
    cat > /tmp/ruststrom-bench.toml << EOF
http_address = "[::]:8000"
https_address = "[::]:8443"

[health_interval]
check_every = 10

[[backend_pools]]
matcher = "Host('localhost')"
addresses = ["127.0.0.1:8080", "127.0.0.1:8081", "127.0.0.1:8082"]
schemes = ["HTTP"]
strategy = { RoundRobin = {} }

[backend_pools.health_config]
slow_threshold = 300
timeout = 500
path = "/"
EOF

    print_success "Configuration created at /tmp/ruststrom-bench.toml"
}

# Start RustStrom
start_ruststrom() {
    print_info "Starting RustStrom..."
    
    if [ ! -f "./target/release/rust-strom" ]; then
        print_error "RustStrom binary not found. Building..."
        cargo build --release
    fi
    
    ./target/release/rust-strom --config /tmp/ruststrom-bench.toml > /tmp/ruststrom.log 2>&1 &
    RUSTSTROM_PID=$!
    echo $RUSTSTROM_PID > /tmp/ruststrom.pid
    
    sleep 3
    print_success "RustStrom started (PID: $RUSTSTROM_PID)"
}

# Stop RustStrom
stop_ruststrom() {
    print_info "Stopping RustStrom..."
    
    if [ -f /tmp/ruststrom.pid ]; then
        PID=$(cat /tmp/ruststrom.pid)
        kill $PID 2>/dev/null || true
        rm /tmp/ruststrom.pid
        print_success "RustStrom stopped"
    fi
}

# Run benchmark
run_benchmark() {
    local NAME=$1
    local URL=$2
    
    echo ""
    echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}  Benchmarking: $NAME${NC}"
    echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
    echo ""
    
    print_info "Warmup (10 seconds)..."
    wrk -t4 -c100 -d10s "$URL" > /dev/null 2>&1
    
    print_info "Running main benchmark..."
    print_info "Duration: $DURATION"
    print_info "Connections: $CONNECTIONS"
    print_info "Threads: $THREADS"
    echo ""
    
    wrk -t$THREADS -c$CONNECTIONS -d$DURATION --latency "$URL"
}

# Run sustained load test
run_sustained_load() {
    echo ""
    echo -e "${BLUE}═══════════════════════════════════════════════════════${NC}"
    echo -e "${BLUE}  Sustained Load Test (5 minutes)${NC}"
    echo -e "${BLUE}═══════════════════════════════════════════════════════${NC}"
    echo ""
    
    wrk -t8 -c500 -d300s http://localhost:8000
}

# Run spike test
run_spike_test() {
    echo ""
    echo -e "${YELLOW}═══════════════════════════════════════════════════════${NC}"
    echo -e "${YELLOW}  Spike Test (5000 connections for 30 seconds)${NC}"
    echo -e "${YELLOW}═══════════════════════════════════════════════════════${NC}"
    echo ""
    
    wrk -t16 -c5000 -d30s http://localhost:8000
}

# Test direct backend
test_direct_backend() {
    print_info "Testing direct backend (no load balancer)..."
    run_benchmark "Direct Backend" "http://localhost:8080"
}

# Test RustStrom
test_ruststrom() {
    print_info "Testing RustStrom..."
    run_benchmark "RustStrom" "http://localhost:8000"
}

# Generate report
generate_report() {
    echo ""
    echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
    echo -e "${GREEN}  Benchmark Complete!${NC}"
    echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
    echo ""
    
    print_info "Results saved to:"
    echo "  - RustStrom logs: /tmp/ruststrom.log"
    echo ""
    
    print_info "Next steps:"
    echo "  1. Compare results with HAProxy"
    echo "  2. Run sustained load test: $0 --sustained"
    echo "  3. Run spike test: $0 --spike"
    echo "  4. Tune configuration for your workload"
    echo ""
}

# Cleanup function
cleanup() {
    print_info "Cleaning up..."
    stop_ruststrom
    stop_backends
    print_success "Cleanup complete"
}

# Trap cleanup on exit
trap cleanup EXIT INT TERM

# Main function
main() {
    print_banner
    
    # Parse arguments
    SUSTAINED=false
    SPIKE=false
    
    for arg in "$@"; do
        case $arg in
            --sustained)
                SUSTAINED=true
                shift
                ;;
            --spike)
                SPIKE=true
                shift
                ;;
            --help)
                echo "Usage: $0 [OPTIONS]"
                echo ""
                echo "Options:"
                echo "  --sustained    Run sustained load test (5 minutes)"
                echo "  --spike        Run spike test (5000 connections)"
                echo "  --help         Show this help message"
                echo ""
                echo "Environment variables:"
                echo "  DURATION       Benchmark duration (default: 60s)"
                echo "  CONNECTIONS    Concurrent connections (default: 1000)"
                echo "  THREADS        Number of threads (default: 8)"
                exit 0
                ;;
        esac
    done
    
    check_wrk
    start_backends
    create_config
    start_ruststrom
    
    echo ""
    print_info "Starting benchmarks in 3 seconds..."
    sleep 3
    
    test_direct_backend
    test_ruststrom
    
    if [ "$SUSTAINED" = true ]; then
        run_sustained_load
    fi
    
    if [ "$SPIKE" = true ]; then
        run_spike_test
    fi
    
    generate_report
}

# Run main
main "$@"
