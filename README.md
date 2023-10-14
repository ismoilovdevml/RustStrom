## TCP/UDP(Layer 4) Load Balancer

### Installer

```bash
curl -sSL https://raw.githubusercontent.com/ismoilovdevml/RustStrom/main/installer.sh | bash
```

### Get Permissions

```bash
sudo groupadd rust-strom
sudo useradd -r -s /bin/false -g rust-strom rust-strom
sudo chmod 750 /etc/rust-strom/rust-strom
sudo chmod 640 /etc/rust-strom/rust-strom/*
```

```bash
sudo nano /etc/systemd/system/rust-strom.service
```

```bash
[Unit]
Description=Load Balancer RS
After=network.target

[Service]
ExecStart=/etc/rust-strom
WorkingDirectory=/opt/rust-strom
Restart=always
User=loadbalancer
Group=loadbalancer
Environment="PATH=/usr/bin:/bin" "LOAD_BALANCER_CONFIG=/etc/rust-strom/loadbalancer.toml"
AmbientCapabilities=CAP_NET_BIND_SERVICE

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable rust-strom
sudo systemctl start rust-strom
sudo systemctl daemon-reload
sudo systemctl restart rust-strom
sudo systemctl status rust-strom
```