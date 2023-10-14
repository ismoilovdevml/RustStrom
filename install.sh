#!/bin/bash
# Clone the git repo
git clone https://github.com/ismoilovdevml/RustStrom.git

# Move to the directory
cd rust-strom

# Build the program (assuming you have Rust and Cargo installed)
cargo build --release

# Move the executable and configuration
sudo mv target/release/rust-strom /usr/local/bin/
sudo mkdir -p /etc/rust-strom
sudo mv path/to/config.toml /etc/rust-strom/config.toml

# Set up user, group, and permissions
sudo addgroup rust-strom
sudo adduser --system --no-create-home --ingroup rust-strom rust-strom
sudo chown rust-strom:rust-strom /usr/local/bin/rust-strom
sudo chown -R rust-strom:rust-strom /etc/rust-strom/
sudo chmod 755 /usr/local/bin/rust-strom

# Create and start the systemd service
echo "[Unit]
Description=Rust Strom Service
After=network.target

[Service]
Type=simple
User=rust-strom
Group=rust-strom
ExecStart=/usr/local/bin/rust-strom --config /etc/rust-strom/config.toml
Restart=on-failure

[Install]
WantedBy=multi-user.target" | sudo tee /etc/systemd/system/rust-strom.service

sudo systemctl enable rust-strom
sudo systemctl start rust-strom
