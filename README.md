## Rust-Strom is a powerful and efficient Load Balancer

### Installation

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

### Configuration

Rust-Strom uses a TOML-based configuration file, typically located at /etc/rust-strom/config.toml. This file allows you to specify:

HTTP and HTTPS binding addresses.
Backend pools, along with matching criteria and addresses.
Load balancing strategies.
Middlewares for additional request processing.
Please refer to the example [config.toml](https://github.com/ismoilovdevml/RustStrom/blob/main/configs/config.toml) provided with the program for a detailed breakdown of available options.