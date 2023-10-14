#!/bin/bash
git clone https://github.com/ismoilovdevml/RustStrom.git

mkdir -p /etc/rust-strom
mkdir -p /opt/rust-strom

cd RustStrom
cp -r rust-strom /usr/bin
cp -r srs/ Cargo.lock Cargo.toml configs/ /opt/rust-strom
cp -r rust-strom.service /etc/systemd/system/
sudo groupadd rust-strom

sudo useradd -r -s /bin/false -g rust-strom rust-strom
sudo chmod 750 /etc/rust-strom/
sudo chmod 640 /etc/rust-strom/*

sudo systemctl enable rust-strom
sudo systemctl start rust-strom
sudo systemctl daemon-reload
sudo systemctl restart rust-strom
sudo systemctl status rust-strom