# cc-connect



# 1、安装cc-connect

使用npm安装cc-connect

```bash
npm install -g cc-connect
```

检查cc-connect是否安装成功：

```bash
cc-connect --version
```



# 2、创建机器人账户

cc-connect目前只支持accesstoken方式接入，需要在 VaChat 控制台机器人列表中点击“管理密钥”生成 API Key，并妥善保存。操作细节参考 [index.md](./index.md) 中的“设置 API Key”。



# 3、配置matrix连接

参考下面的配置matrix连接信息，配置文件在 `~/.cc-connect/config.toml` ，请注意windows环境下的路径写法。

```
[[projects]]
name = "vachat-server"

[projects.agent]
type = "claudecode"

[projects.agent.options]
mode = "default"
work_dir = "d:\\workspace\\vachat\\vachat-server"

[[projects.platforms]]
type = "matrix"

[projects.platforms.options]
homeserver = "https://chat.zimu.pub"
access_token = "10384f****59227d"

# ── Optional settings ────────────────────────────────────────
# user_id = "@bot:matrix.org"        # auto-detected if omitted
# allow_from = "*"                   # "*" = all users, or "id1,id2"
# auto_join = true                   # auto-accept room invites (default: true)
# auto_verify = true                 # auto-accept SAS key verification (default: true)
# cross_signing_password = ""        # bot account password for cross-signing setup (one-time)
# share_session_in_channel = false   # true = all users share one session per room
# group_reply_all = false            # true = respond to all messages in group rooms
# proxy = ""                         # HTTP/SOCKS5 proxy, e.g. "http://proxy:8080"
```



# 4、运行cc-connect

使用命令cc-connect启动服务

```
cc-connect
```

如果要指定配置文件，可以使用下面的命令：

```
cc-connect -config /path/to/config.toml
```
