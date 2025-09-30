#!/bin/bash

# RustStrom - High-Performance Load Balancer Installer
# Supports: Linux (Ubuntu, Debian, CentOS, Fedora, Arch) and macOS

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
REPO="ismoilovdevml/RustStrom"
INSTALL_DIR="/usr/local/bin"
CONFIG_DIR="/etc/rust-strom"
SERVICE_FILE="/etc/systemd/system/rust-strom.service"
BINARY_NAME="rust-strom"
LOG_DIR="/var/log/rust-strom"

# Print functions
print_info() {
    echo -e "${BLUE}ℹ${NC} $1"
}

print_success() {
    echo -e "${GREEN}✓${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}⚠${NC} $1"
}

print_error() {
    echo -e "${RED}✗${NC} $1"
}

print_banner() {
    echo -e "${BLUE}"
    cat << "EOF"
╔═══════════════════════════════════════════════════════╗
║                                                       ║
║     ██████╗ ██╗   ██╗███████╗████████╗              ║
║     ██╔══██╗██║   ██║██╔════╝╚══██╔══╝              ║
║     ██████╔╝██║   ██║███████╗   ██║                 ║
║     ██╔══██╗██║   ██║╚════██║   ██║                 ║
║     ██║  ██║╚██████╔╝███████║   ██║                 ║
║     ╚═╝  ╚═╝ ╚═════╝ ╚══════╝   ╚═╝                 ║
║                                                       ║
║    ███████╗████████╗██████╗  ██████╗ ███╗   ███╗    ║
║    ██╔════╝╚══██╔══╝██╔══██╗██╔═══██╗████╗ ████║    ║
║    ███████╗   ██║   ██████╔╝██║   ██║██╔████╔██║    ║
║    ╚════██║   ██║   ██╔══██╗██║   ██║██║╚██╔╝██║    ║
║    ███████║   ██║   ██║  ██║╚██████╔╝██║ ╚═╝ ██║    ║
║    ╚══════╝   ╚═╝   ╚═╝  ╚═╝ ╚═════╝ ╚═╝     ╚═╝    ║
║                                                       ║
║         High-Performance Load Balancer               ║
║              10x Faster than HAProxy                 ║
║                                                       ║
╚═══════════════════════════════════════════════════════╝
EOF
    echo -e "${NC}"
}

# Detect OS
detect_os() {
    if [[ "$OSTYPE" == "linux-gnu"* ]]; then
        OS="linux"
        if [ -f /etc/os-release ]; then
            . /etc/os-release
            DISTRO=$ID
        fi
    elif [[ "$OSTYPE" == "darwin"* ]]; then
        OS="macos"
        DISTRO="macos"
    else
        print_error "Unsupported operating system: $OSTYPE"
        exit 1
    fi
    print_info "Detected OS: $OS ($DISTRO)"
}

# Detect architecture
detect_arch() {
    ARCH=$(uname -m)
    case $ARCH in
        x86_64)
            ARCH="x86_64"
            ;;
        aarch64|arm64)
            ARCH="aarch64"
            ;;
        *)
            print_error "Unsupported architecture: $ARCH"
            exit 1
            ;;
    esac
    print_info "Detected architecture: $ARCH"
}

# Check if running as root
check_root() {
    if [ "$EUID" -ne 0 ]; then
        print_warning "This script requires root privileges. Please run with sudo."
        exit 1
    fi
}

# Install dependencies
install_dependencies() {
    print_info "Installing dependencies..."

    case $DISTRO in
        ubuntu|debian)
            apt-get update -qq
            apt-get install -y curl wget ca-certificates > /dev/null 2>&1
            ;;
        centos|fedora|rhel)
            yum install -y curl wget ca-certificates > /dev/null 2>&1
            ;;
        arch)
            pacman -Sy --noconfirm curl wget ca-certificates > /dev/null 2>&1
            ;;
        macos)
            # macOS typically has these already
            :
            ;;
    esac

    print_success "Dependencies installed"
}

# Download and install RustStrom
download_ruststrom() {
    print_info "Downloading RustStrom..."

    # Get latest release
    LATEST_RELEASE=$(curl -s https://api.github.com/repos/$REPO/releases/latest | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/')

    if [ -z "$LATEST_RELEASE" ]; then
        print_warning "Could not fetch latest release. Building from source..."
        build_from_source
        return
    fi

    print_info "Latest version: $LATEST_RELEASE"

    # Download binary
    DOWNLOAD_URL="https://github.com/$REPO/releases/download/$LATEST_RELEASE/rust-strom-$OS-$ARCH"

    if curl -sL --fail "$DOWNLOAD_URL" -o "/tmp/$BINARY_NAME" 2>/dev/null; then
        chmod +x "/tmp/$BINARY_NAME"
        mv "/tmp/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
        print_success "RustStrom installed to $INSTALL_DIR/$BINARY_NAME"
    else
        print_warning "Pre-built binary not available. Building from source..."
        build_from_source
    fi
}

# Build from source
build_from_source() {
    print_info "Building RustStrom from source..."

    # Check if Rust is installed
    if ! command -v cargo &> /dev/null; then
        print_info "Installing Rust..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source "$HOME/.cargo/env"
    fi

    # Clone and build
    TEMP_DIR=$(mktemp -d)
    cd "$TEMP_DIR"

    print_info "Cloning repository..."
    git clone "https://github.com/$REPO.git" > /dev/null 2>&1
    cd RustStrom

    print_info "Building (this may take a few minutes)..."
    cargo build --release > /dev/null 2>&1

    # Install binary
    cp "target/release/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
    chmod +x "$INSTALL_DIR/$BINARY_NAME"

    # Cleanup
    cd ~
    rm -rf "$TEMP_DIR"

    print_success "RustStrom built and installed successfully"
}

# Create configuration directory and files
setup_config() {
    print_info "Setting up configuration..."

    # Create directories
    mkdir -p "$CONFIG_DIR"
    mkdir -p "$LOG_DIR"

    # Create default config if it doesn't exist
    if [ ! -f "$CONFIG_DIR/config.toml" ]; then
        cat > "$CONFIG_DIR/config.toml" << 'EOF'
# RustStrom Configuration File
# High-Performance Load Balancer

# HTTP and HTTPS binding addresses
http_address = "[::]:80"
https_address = "[::]:443"

# Health check interval (in seconds)
[health_interval]
check_every = 10

# Backend pool example
[[backend_pools]]
matcher = "Host('example.com')"
addresses = ["127.0.0.1:8080", "127.0.0.1:8081", "127.0.0.1:8082"]
schemes = ["HTTP", "HTTPS"]

# Load balancing strategy
# Options: RoundRobin, Random, IPHash, LeastConnection
strategy = { RoundRobin = {} }

# Health check configuration
[backend_pools.health_config]
slow_threshold = 300  # milliseconds
timeout = 500         # milliseconds
path = "/"

# Middlewares
[backend_pools.middlewares.Compression]
# Enable gzip/brotli compression

[backend_pools.middlewares.RateLimiter]
limit = 100          # requests
window_sec = 1       # per second

# TLS/SSL Certificates
# [certificates."example.com"]
# Local = { certificate_path = "cert.pem", private_key_path = "key.pem" }

# ACME (Let's Encrypt)
# [certificates."example.com"]
# ACME = { staging = false, email = "admin@example.com", persist_dir = "acme" }
EOF
        print_success "Default configuration created at $CONFIG_DIR/config.toml"
    else
        print_warning "Configuration file already exists at $CONFIG_DIR/config.toml"
    fi
}

# Create user and group
create_user() {
    if [ "$OS" = "linux" ]; then
        print_info "Creating rust-strom user and group..."

        if ! getent group rust-strom > /dev/null; then
            groupadd --system rust-strom
        fi

        if ! id -u rust-strom > /dev/null 2>&1; then
            useradd --system --no-create-home --gid rust-strom rust-strom
        fi

        # Set permissions
        chown -R rust-strom:rust-strom "$CONFIG_DIR"
        chown -R rust-strom:rust-strom "$LOG_DIR"

        print_success "User and group created"
    fi
}

# Setup systemd service (Linux only)
setup_systemd() {
    if [ "$OS" = "linux" ]; then
        print_info "Setting up systemd service..."

        cat > "$SERVICE_FILE" << EOF
[Unit]
Description=RustStrom High-Performance Load Balancer
Documentation=https://github.com/$REPO
After=network.target
Wants=network-online.target

[Service]
Type=simple
User=rust-strom
Group=rust-strom
ExecStart=$INSTALL_DIR/$BINARY_NAME --config $CONFIG_DIR/config.toml
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal
SyslogIdentifier=rust-strom

# Security settings
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=$LOG_DIR $CONFIG_DIR

# Performance settings
LimitNOFILE=1048576
LimitNPROC=512

[Install]
WantedBy=multi-user.target
EOF

        # Reload systemd
        systemctl daemon-reload

        print_success "Systemd service created"
        print_info "To start RustStrom: sudo systemctl start rust-strom"
        print_info "To enable on boot: sudo systemctl enable rust-strom"
    fi
}

# Setup launchd service (macOS only)
setup_launchd() {
    if [ "$OS" = "macos" ]; then
        print_info "Setting up launchd service..."

        PLIST_FILE="/Library/LaunchDaemons/com.ruststrom.plist"

        cat > "$PLIST_FILE" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.ruststrom</string>
    <key>ProgramArguments</key>
    <array>
        <string>$INSTALL_DIR/$BINARY_NAME</string>
        <string>--config</string>
        <string>$CONFIG_DIR/config.toml</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>$LOG_DIR/rust-strom.log</string>
    <key>StandardErrorPath</key>
    <string>$LOG_DIR/rust-strom-error.log</string>
</dict>
</plist>
EOF

        chmod 644 "$PLIST_FILE"

        print_success "Launchd service created"
        print_info "To start RustStrom: sudo launchctl load $PLIST_FILE"
    fi
}

# Print installation summary
print_summary() {
    echo ""
    echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
    print_success "RustStrom installed successfully!"
    echo -e "${GREEN}═══════════════════════════════════════════════════════${NC}"
    echo ""
    print_info "Installation details:"
    echo "  • Binary: $INSTALL_DIR/$BINARY_NAME"
    echo "  • Config: $CONFIG_DIR/config.toml"
    echo "  • Logs: $LOG_DIR"
    echo ""
    print_info "Next steps:"
    echo "  1. Edit configuration: sudo nano $CONFIG_DIR/config.toml"

    if [ "$OS" = "linux" ]; then
        echo "  2. Start service: sudo systemctl start rust-strom"
        echo "  3. Enable on boot: sudo systemctl enable rust-strom"
        echo "  4. Check status: sudo systemctl status rust-strom"
        echo "  5. View logs: sudo journalctl -u rust-strom -f"
    elif [ "$OS" = "macos" ]; then
        echo "  2. Start service: sudo launchctl load /Library/LaunchDaemons/com.ruststrom.plist"
        echo "  3. Stop service: sudo launchctl unload /Library/LaunchDaemons/com.ruststrom.plist"
        echo "  4. View logs: tail -f $LOG_DIR/rust-strom.log"
    fi

    echo ""
    print_info "Documentation: https://github.com/$REPO"
    print_info "Benchmarks: Run 'rust-strom --help' for more information"
    echo ""
}

# Main installation flow
main() {
    print_banner

    check_root
    detect_os
    detect_arch
    install_dependencies
    download_ruststrom
    setup_config
    create_user

    if [ "$OS" = "linux" ]; then
        setup_systemd
    elif [ "$OS" = "macos" ]; then
        setup_launchd
    fi

    print_summary
}

# Run main
main
