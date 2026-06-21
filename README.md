# VaChat：打造专属AI虚拟助手

AI 时代，OpenClaw、QwenPaw、Hermes、Claude Code 等优秀智能体（Agent）已成为我们工作生活中不可或缺的助手。但你是否厌倦了将它们分散在微信、Telegram 等社交软件中？如果能有一个完全私有、安全且不受干扰的应用来集中管理这些智能体，体验将会截然不同。

VaChat（Virtual Assistant Chat）是基于开源项目 [VoceChat](https://doc.voce.chat/zh-cn/) 二次开发的私人虚拟助手平台，正是为了解决这一痛点而来。它致力于满足个人用户对轻量级、私有化 AI 虚拟助手的管理需求，让你的 AI 助手更加井井有条。

# **一、关于VaChat**

你可能会问，市面上不是已经有 Telegram、WhatsApp 这种可以接入机器人的应用吗？或者企业微信、钉钉也能用。

确实如此，但它们各有痛点：

- **国际应用（Telegram等）：** 在中国大陆访问体验极差，且数据在境外。
- **办公软件（企微/钉钉）：** 过于臃肿，且主要面向办公场景，缺乏私密性。
- **其他开源方案（如Rocket.Chat等）：** 部署复杂，配置繁琐，对个人用户不友好。

**VaChat** 的诞生正是为了解决这些问题。它继承了VoceChat的**极致轻量**和**多端共通**特性，并进一步精简了代码和逻辑，只保留了核心聊天逻辑。另外通过增加对**Matrix协议**的支持实现了与市面上几乎所有主流 AI 智能体的无缝对接。

简单来说，VaChat是一个完全私有、安全、且只属于你一个人的AI 助理中心。

![image-20260511205540](./assets/image-20260511205540.png)



## 演示1：Web端指挥远程Claude Code完成编码任务

![claude](./assets/claude.gif)



## 演示2：移动端与QwenPaw对话

![qwenpaw](./assets/qwenpaw.gif)



# **二、快速部署：Docker 一键启动**

部署CocoChat非常简单，你只需要一台云服务器（阿里云、腾讯云、华为云均可，推荐 Debian 12 或 Ubuntu 系统）。当然，如果你不需要公网访问，局域网内服务器也可以。以下以 Ubuntu 24.04 为例：

## **1. 安装 Docker 环境**

```
sudo apt update
sudo apt install -y ca-certificates curl
sudo apt install docker.io
```

## **2. 拉取镜像**

拉取cocochat-server最新镜像：

```
docker pull zimucode/cocochat-server:latest
```

如果镜像无法拉取，你可以考虑通过国内镜像进行拉取。

```
docker pull docker.1ms.run/zimucode/cocochat-server:latest
docker tag  docker.1ms.run/zimucode/cocochat-server:latest  zimucode/cocochat-server
```

## **3. 运行容器**

推荐将数据挂载到本地目录，防止容器删除后数据丢失：

```shell
# 运行容器
docker run -d --restart=always \
  -p3000:3000 \
  --name cocochat-server \
  -v ./data:/home/cocochat-server/data \
  zimucode/cocochat-server:latest
```

**提示：** 默认端口是 `3000`，如果需要修改，可以调整 `-p` 后面的端口号。



# **三、手动编译**

如果你需要手动编译，请参考 github 项目地址。



# **四、初始化与使用**

部署完成后，访问 `http://你的服务器IP:3000` 即可进入初始化页面。

## **1、初始化**

输入服务器名称、管理员邮箱和密码，即可完成安装。

![image-20250411205918](./assets/image-20250411205918.png)



## **2、用户注册**

首页点击注册。注意，VaChat 不强制验证邮箱真实性，只要格式正确即可注册登录。

![image-20250511210039](./assets/image-20250511210039.png)



## **3、WEB端**

在浏览器中输入服务器地址和端口号，登录页面输入邮箱账号和密码，登录成功即可使用。

![image-20250511210136](./assets/image-20250511210136.png)



## **4、移动端**

安卓用户可下载 APK 安装包（iOS 版本目前暂未编译，后续会跟进）。

安卓APK下载地址： 

https://chat.zimu.pub/apk/cocochat-v0.1.1.apk

首页输入服务器的地址和端口号，然后在登录页面输入邮箱和密码完成登录即可使用。

![image-20250511210258](./assets/image-20250511210258.png)



# **五、接入 AI 智能体**

VaChat对VoceChat做了二次开发，可以快速接入各种 AI Agent。其实现原理是通过配置智能体的 **Matrix 频道**，将机器人接入到 VaChat 服务中。

这里以 **QwenPaw** 为例，其他智能体（OpenClaw, Hermes 等）的配置逻辑大同小异。

## **1、创建机器人**

以管理员身份登录 VaChat 控制台，进入 **“机器人 & Webhook”** 菜单，点击创建机器人。

- **名称：** 可以随便起，比如 `QwenBot，Agent使用用户密码方式接入时会用到。`
- **密码：**机器人密码，当客户端使用账号密码方式接入时使用，如果使用token方式接入，这个密码就不起作用。
- **Webhook URL：** 可选，用于接收推送数据，在matrix协议连接机器人的情况下没有用。



## **2、设置密码或API Key**

如果智能体客户端使用token方式接入，需要创建ApiKey，使用用户名密码方式接入不需要，会自动创建ApiKey。

机器人创建成功后，点击“新增API Key”创建机器人的ApiKey，请妥善保管这个信息，后续matrix接入的时候会用到。



## **3、配置Matrix频道**

下面以QwenPaw为例展示配置matrix频道的方法，QwenPaw有两种方式配置 Matrix 频道，其他智能体的配置方式可参见本节最后部分。

### **方式一： 在 Console 中配置**

前往 **控制 → 频道**，点击 **Matrix**，启用后填写：

- **Homeserver URL:** 填写你的 CocoChat 服务器地址，格式为 `https://你的域名或IP:端口`。
- **User ID:** 填写刚才创建机器人得到的 ID。
- **Access Token:** 填写刚才生成的 Token。

![image-20250511210606](./assets/image-20250511210606.png)

### **方式二：编辑配置文件 (agent.json)**

如果你是在本地运行智能体，找到 `agent.json` 文件（路径通常为 `~/.qwenpaw/workspaces/default/agent.json`），在 `channels` 中添加 matrix 配置：

```
"matrix": {
  "enabled": true,
  "bot_prefix": "[BOT]",
  "homeserver": "https://matrix.org",
  "user_id": "@mybot:matrix.org",
  "access_token": "syt_..."
}
```

保存后，智能体通常会自动重载配置。其他智能体的配置方式大同小异，

注意：目前VaChat仅支持一对一聊天加密，不支持群组聊天加密，因此开启e2ee加密选项的情况下可能有不稳定的情况。



## **4、开始对话**

配置成功后，你的机器人就会出现在好友列表中。直接点击对话，发送“Hello”，如果能收到回复，说明链路已经打通！

![image-20250511210720](./assets/image-20250511210720.png)



## 5、其他智能体配置说明

### QwenPaw

https://qwenpaw.agentscope.io/docs/channels#Matrix

### OpenClaw

https://docs.openclaw.ai/zh-CN/channels/matrix

### Nanobot

https://github.com/HKUDS/nanobot

### Hermes Agent

https://hermesagent.org.cn/docs/user-guide/messaging/matrix

### ZeroClaw

https://github.com/zeroclaw-labs/zeroclaw/blob/master/docs/i18n/zh-CN/security/matrix-e2ee-guide.zh-CN.md



# **六、使用cc-connect接入ClaudeCode等智能体**

可以使用 [cc-connect](https://github.com/chenhg5/cc-connect) 将cluade code、codex、gemini cli、opencode等智能体接入cocochat，但cc-connect的当前版本尚不支持matrix协议，需要自行编译 [cc-connect-matrix](https://github.com/rablwupei/cc-connect-matrix) 这个feature分支才能通过matrix协议接入cocochat。

编译完成后通过下面的matrix协议配置参数来接入cocochat。

```
[[projects.platforms]]
type = "matrix"

[projects.platforms.options]
homeserver = "https://matrix-home-server.com"
access_token = "syt_xxx_xxx"

# Optional settings
# user_id = "@bot:matrix.org"           # auto-detected if omitted
# allow_from = "*"                      # "*" = all users, or "id1,id2"
# auto_join = true                      # auto-accept room invites (default: true)
# auto_verify = true                    # auto-accept SAS verification (default: true)
# cross_signing_password = ""           # bot password for cross-signing setup (one-time)
# share_session_in_channel = false      # all users share one session per room
# group_reply_all = false               # respond to all messages in group rooms
# proxy = ""                            # HTTP/SOCKS5 proxy
```
