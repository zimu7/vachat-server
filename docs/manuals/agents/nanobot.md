# Nanobot

**nanobot 配置说明：** [https://github.com/HKUDS/nanobot/blob/main/docs/chat-apps.md](https://github.com/HKUDS/nanobot/blob/main/docs/chat-apps.md)

# 1、安装 nanobot

```bash
pip install nanobot-ai
```

执行以下命令验证是否安装成功：

```bash
nanobot --version
```

能正常输出版本号，即表示安装完成。

# 2、启用 Matrix 插件

先启用 Matrix 插件：

```bash
nanobot plugins enable matrix
```

再用以下命令确认插件已正确安装：

```bash
nanobot plugins list
```

注意：Windows 上默认禁用 Matrix 端到端加密（E2EE），因为 `matrix-nio[e2e]` 依赖 `python-olm`，而该库没有预编译的 Windows wheel。如需使用 E2EE，请在 macOS、Linux 或 WSL2 环境下运行。

# 3、创建机器人账户

机器人账户在 VaChat 中创建：以管理员身份登录控制台，进入 **设置 → 成员**，新增成员时勾选“设为机器人”。详细步骤参考 [index.md](./index.md)。

创建后请记下机器人的**用户 ID**（如 `@nanobot:zimu.pub`）和**密码**，下一步配置 Matrix 连接时会用到。

# 4、配置 Matrix 连接

nanobot 的配置文件为 `~/.nanobot/config.json`（JSON 格式），将 Matrix 配置合并到 `channels` 字段下即可。Matrix 支持以下两种鉴权方式：

## 方式 A：密码登录（推荐）

```json
{
  "channels": {
    "matrix": {
      "enabled": true,
      "homeserver": "https://chat.zimu.pub",
      "userId": "@nanobot:zimu.pub",
      "password": "mypasswordhere",
      "e2eeEnabled": false,
      "sasVerification": true,
      "allowFrom": ["@test:zimu.pub"],
      "groupPolicy": "open",
      "groupAllowFrom": [],
      "allowRoomMentions": false,
      "maxMediaBytes": 20971520
    }
  }
}
```

## 方式 B：访问令牌验证（已废弃）

```json
{
  "channels": {
    "matrix": {
      "enabled": true,
      "homeserver": "https://chat.zimu.pub",
      "userId": "@nanobot:zimu.pub",
      "deviceId": "nanobot",
      "accessToken": "myaccesstoken",
      "e2eeEnabled": false,
      "sasVerification": true,
      "allowFrom": ["@test:zimu.pub"],
      "groupPolicy": "open",
      "groupAllowFrom": [],
      "allowRoomMentions": false,
      "maxMediaBytes": 20971520
    }
  }
}
```

注意：出于兼容性考虑，`accessToken` 和 `deviceId` 仍可使用，但为保证加密功能稳定运行，推荐改用密码登录。若同时提供了 `password`，则 `accessToken` 和 `deviceId` 会被忽略。

**字段说明：**

| 字段 | 说明 |
| --- | --- |
| `homeserver` | Matrix 服务器地址，VaChat 中即 `https://chat.zimu.pub` |
| `userId` | 机器人用户 ID，如 `@nanobot:zimu.pub` |
| `password` | 机器人密码（与 VaChat 创建机器人时设置的密码一致） |
| `accessToken` / `deviceId` | 访问令牌与设备 ID，仅用于兼容旧配置；提供 `password` 时将被忽略 |
| `allowFrom` | 允许与机器人交互的用户 ID 列表；留空拒绝所有，`["*"]` 表示允许所有人。私聊中仅该列表内用户可触发 |
| `groupPolicy` | 群聊响应策略：`open`（默认，响应全部消息）、`mention`（仅响应 @ 提及）、`allowlist`（仅响应白名单房间） |
| `groupAllowFrom` | 房间白名单，仅在 `groupPolicy` 为 `allowlist` 时生效 |
| `allowRoomMentions` | 在 `mention` 模式下是否接受 `@room` 整房提及 |
| `e2eeEnabled` | 是否启用端到端加密（默认 `true`）；设为 `false` 仅使用明文。Windows 不支持，见第 2 节 |
| `sasVerification` | 是否自动完成允许用户的 SAS 设备验证（默认 `false`），适用于 Element X 等无法手动信任第三方设备的客户端 |
| `maxMediaBytes` | 单条消息附件大小上限（默认 20MB）；设为 `0` 禁止所有媒体 |

# 5、启动服务

执行以下命令启动 nanobot 服务：

```bash
nanobot gateway -v
```

启动后机器人会连接 homeserver 并开始监听消息。若启用了 E2EE，请保持 `matrix-store` 持久化，避免重启后会话状态丢失。
