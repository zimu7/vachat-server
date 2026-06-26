



# 部署服务

config / config.toml



```
[system]
data_dir = "./data"
wwwroot_dir = "./wwwroot"
token_expiry_seconds = 300
refresh_token_expiry_seconds = 604800
log_level = "debug"

[network]
bind = "0.0.0.0:443"
domain = "chat.zimu.pub"
matrix_domain= "zimu.pub"
frontend_url = "http://127.0.0.1:3000"
enable_swagger = false

[network.tls]
type = "certificate"
cert_path = "./cert/fullchain.pem"
key_path = "./cert/privkey.pem"
#path = "./cert/"

```



# self_signed



# certificate



## 手动更新证书

```
sudo apt-get install certbot

certbot certonly --standalone -d chat.zimu.pub
```



如果成功，证书应该会保存在下面的地址：

```
Successfully received certificate.
Certificate is saved at: /etc/letsencrypt/live/chat.zimu.pub/fullchain.pem
Key is saved at:         /etc/letsencrypt/live/chat.zimu.pub/privkey.pem
This certificate expires on 2026-09-24.
These files will be updated when the certificate renews.
Certbot has set up a scheduled task to automatically renew this certificate in the background.
```



拷贝证书，其中 ~/vachat-server/cert 是vachat-server服务所在的地址。

```
cp /etc/letsencrypt/live/chat.zimu.pub/fullchain.pem  ~/vachat-server/cert/
cp /etc/letsencrypt/live/chat.zimu.pub/privkey.pem  ~/vachat-server/cert/
```



# acme_http_01



# acme_tls_alpn_01





