
## TCP/UDP(Layer 4) Load Balancer


### Installer

```bash
curl -sSL https://raw.githubusercontent.com/ismoilovdevml/load-balancer-rs/main/installer.sh | bash
```

### Get Permissions
```bash
sudo groupadd loadbalancer
sudo useradd -r -s /bin/false -g loadbalancer loadbalancer
sudo chmod 750 /etc/load-balancer-rs/load-balancer-rs
sudo chmod 640 /etc/load-balancer-rs/load-balancer-rs/*
```



```bash
sudo nano /etc/systemd/system/load-balancer-rs.service
```


```bash
[Unit]
Description=Load Balancer RS
After=network.target

[Service]
ExecStart=/etc/load-balancer-rs/load-balancer-rs
WorkingDirectory=/opt/load-balancer-rs
Restart=always
User=loadbalancer
Group=loadbalancer
Environment="PATH=/usr/bin:/bin" "LOAD_BALANCER_CONFIG=/etc/load-balancer-rs/loadbalancer.toml"
AmbientCapabilities=CAP_NET_BIND_SERVICE

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable load-balancer-rs
sudo systemctl start load-balancer-rs
sudo systemctl daemon-reload
sudo systemctl restart load-balancer-rs
sudo systemctl status load-balancer-rs
```