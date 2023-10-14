## TCP/UDP(Layer 4) Load Balancer

### Installer

```bash
curl -sSL https://raw.githubusercontent.com/ismoilovdevml/RustStrom/main/installer.sh | bash
chmod +x install.sh
./install.sh
```

### Creating a New User and Group for the Program

```bash
sudo addgroup rust-strom
sudo adduser --system --no-create-home --ingroup rust-strom rust-strom
```

### Setting Permissions
```bash
sudo mv /path/to/your/executable /usr/local/bin/rust-strom
sudo mkdir -p /etc/rust-strom
sudo mv /path/to/your/config.toml /etc/rust-strom/config.toml

sudo chown rust-strom:rust-strom /usr/local/bin/rust-strom
sudo chown -R rust-strom:rust-strom /etc/rust-strom/
sudo chmod 755 /usr/local/bin/rust-strom
```
### Creating a systemd Service

```bash
sudo nano /etc/systemd/system/rust-strom.service
```

```bash
[Unit]
Description=Rust Strom Service
After=network.target

[Service]
Type=simple
User=rust-strom
Group=rust-strom
ExecStart=/usr/local/bin/rust-strom --config /etc/rust-strom/config.toml
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

### Enable and start the service
```bash
sudo systemctl enable rust-strom
sudo systemctl start rust-strom


sudo systemctl daemon-reload
sudo systemctl restart rust-strom
sudo systemctl status rust-strom
```