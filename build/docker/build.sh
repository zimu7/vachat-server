#!/bin/bash

VERSION=v0.1.1

cd ../../

# build x86_64
docker run --rm -it \
  -v "$(pwd)":/home/rust/src \
  -v "$(pwd)/config/cargo.toml":/root/.cargo/config.toml \
  -w /home/rust/src \
  clux/muslrust:stable cargo build --release


if [ -f ./target/x86_64-unknown-linux-musl/release/vachat-server ]; then

  cp -rf ./target/x86_64-unknown-linux-musl/release/vachat-server build/docker/vachat-server
  cp -rf config build/docker/

  cd build/docker

  # sudo docker login -u vachat -p xxxxxx
  # build amd64
  docker build --platform=linux/amd64 -t vachat-server:latest -t vachat-server:$VERSION .
  docker tag vachat-server:latest zimucode/vachat-server:latest
  docker tag vachat-server:$VERSION zimucode/vachat-server:$VERSION
  docker push zimucode/vachat-server:$VERSION
  docker push zimucode/vachat-server:latest

  # build arrch64
  # cp -rf ./target/aarch64-unknown-linux-musl/release/vachat-server build/docker/vachat-server
  # docker build --platform=linux/arm64 -t vachat-server:latest-arm64 .
  # docker tag vachat-server:latest-arm64 zimucode/vachat-server:latest-arm64
  # docker push zimucode/vachat-server:latest-arm64
  #rm -rf  ./config

fi
