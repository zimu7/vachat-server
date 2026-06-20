#!/bin/bash

mkdir -p ~/.vachat-server/data/cert
docker stop vachat-server
docker rm vachat-server
docker pull winbomb/vachat-server:latest
docker run -d --restart=always \
  -p 443:443 \
  --name vachat-server \
  -v ~/.vachat-server/data:/home/vachat-server/data \
  winbomb/vachat-server:latest \
  --network.bind "0.0.0.0:443" \
  --network.domain "chat.domain.com" \
  --network.tls.type "acme_tls_alpn_01" \
  --network.tls.acme.directory_url "https://acme-v02.api.letsencrypt.org/directory" \
  --network.tls.acme.cache_path "/home/vachat-server/data/cert"
docker logs -f vachat-server
