# 安装

部署vachat非常简单，你只需要一台云服务器（阿里云、腾讯云、华为云均可，推荐 Debian 12 或 Ubuntu 系统）。当然，如果你不需要公网访问，局域网内服务器也可以。

以下以 Ubuntu 24.04 为例：

## **1. 安装 Docker 环境**

```
sudo apt update
sudo apt install -y ca-certificates curl
sudo apt install docker.io
```

## **2. 拉取镜像**

拉取vachat-server最新镜像：

```
docker pull zimucode/vachat-server:latest
```

如果镜像无法拉取，你可以考虑通过国内镜像进行拉取。

```
docker pull docker.1ms.run/zimucode/vachat-server:latest
docker tag  docker.1ms.run/zimucode/vachat-server:latest  zimucode/vachat-server
```

## **3. 运行容器**

推荐将数据挂载到本地目录，防止容器删除后数据丢失：

```shell
# 运行容器
docker run -d --restart=always \
  -p3000:3000 \
  --name vachat-server \
  -v ./data:/home/vachat-server/data \
  zimucode/vachat-server:latest
```

**提示：** 默认端口是 `3000`，如果需要修改，可以调整 `-p` 后面的端口号。

