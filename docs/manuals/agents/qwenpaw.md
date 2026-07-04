# Qwenpaw

QwenPaw官方文档：[https://qwenpaw.agentscope.io/docs/intro](https://qwenpaw.agentscope.io/docs/intro)



# 1、安装qwenpaw

如果你更习惯自行管理 Python 环境（需 Python >= 3.10, < 3.14）：

```
pip install qwenpaw
```

可选：先创建并激活虚拟环境再安装（`python -m venv .venv`，Linux/macOS 下 `source .venv/bin/activate`，Windows 下 `.venv\Scripts\Activate.ps1`）。安装后会提供 `qwenpaw` 命令。



# 2、创建机器人账户

qwenpaw目前好像只支持accesstoken方式接入，需要在 VaChat 控制台机器人列表中点击“管理密钥”生成 API Key，并妥善保存。操作细节参考 [index.md](./index.md) 中的“设置 API Key”。



# 3、配置matrix频道

**方式一：** 在 Console 中配置

前往 **控制 → 频道**，点击 **Matrix**，启用后填写：

- **Homeserver URL** — 例如 `https://matrix.org`
- **User ID** — 例如 `@mybot:matrix.org`
- **Access Token** — 上面复制的 Token（以密码框形式显示）



**方式二：** 编辑智能体工作区的 `agent.json`

在 `agent.json`（如 `~/.qwenpaw/workspaces/default/agent.json`）中找到 `channels.matrix`：

```
"matrix": {
  "enabled": true,
  "bot_prefix": "qwenpaw",
  "homeserver": "https://chat.zimu.pub",
  "user_id": "@test:zimu.pub",
  "access_token": "syt_..."
}
```

主要就是上面这几个字段，其他一些字段可以暂时不用管。

**Matrix 专属字段说明：**

| 字段           |  类型  |    默认值    |                     说明                     |
| :------------- | :----: | :----------: | :------------------------------------------: |
| `homeserver`   | string | `""`（必填） | Matrix 服务器地址（如 `https://matrix.org`） |
| `user_id`      | string | `""`（必填） |   机器人 User ID（如 `@mybot:matrix.org`）   |
| `access_token` | string | `""`（必填） |   机器人的 Access Token（以 `syt_` 开头）    |

保存后，若 QwenPaw 已在运行，频道会自动重载。



# 4、启动服务

请用下面的命令启动服务

```
qwenpaw app
```

服务默认监听 `127.0.0.1:8088`。若已配置频道，QwenPaw 会在对应 app 内回复；若尚未配置，也可先完成本节再前往频道配置。

可以通过 `qwenpaw app -h` 查看启动的参数，如果希望打印一些日志信息，可加上 --log-level 参数，例如。

```
qwenpaw app --log-level debug
```



### 注意事项

- Matrix 频道当前**仅支持文本消息**（不支持图片/文件附件）。
- 机器人只能接收已加入房间的消息，发消息前请先邀请机器人进入对应房间。
- 如使用自建服务器，将 `homeserver` 设置为你的服务器地址（例如 `https://matrix.example.com`）。
- 注意qwenpaw是支持多智能体的，因此需要配置的是 /.qwenpaw/workspaces/{AGENT_NAME}/agent.json 文件。
