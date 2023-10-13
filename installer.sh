#!/bin/bash
git clone https://github.com/ismoilovdevml/load-balancer-rs.git

mkdir -p /etc/load-balancer-rs
mkdir -p /opt/load-balancer-rs

cd load-balancer-rs
cp -r load-balancer-rs /usr/bin
cp -r srs/ Cargo.lock Cargo.toml configs/ /opt/load-balancer-rs
cp -r load-balancer-rs.service /etc/systemd/system/
sudo groupadd loadbalancer

sudo useradd -r -s /bin/false -g loadbalancer loadbalancer
sudo chmod 750 /etc/load-balancer-rs/
sudo chmod 640 /etc/load-balancer-rs/*

sudo systemctl enable load-balancer-rs
sudo systemctl start load-balancer-rs
sudo systemctl daemon-reload
sudo systemctl restart load-balancer-rs
sudo systemctl status load-balancer-rs