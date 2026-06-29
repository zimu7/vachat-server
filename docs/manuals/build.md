# 编译

本文说明 VaChat 各端应用的编译方式。当前 `vachat-server` 仓库包含服务端源码、Docker 构建文件，以及已经编译好的 Web 静态资源 `wwwroot/`；如果需要重新编译 Web 或移动端，需要先准备对应客户端源码仓库，再把产物放回服务端使用。

## vachat-server 服务端

### 1. 安装基础环境

推荐在 Ubuntu 22.04/24.04 或 Debian 12 上编译。

```bash
sudo apt update
sudo apt install -y build-essential pkg-config curl git
```

安装 Rust 工具链：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustc --version
cargo --version
```

如果网络访问 crates.io 较慢，可以按需配置 Cargo 镜像。项目中已有示例配置：

```text
config/cargo.toml
```

### 2. 编译调试版本

在项目根目录执行：

```bash
cargo build
```

编译产物位于：

```text
target/debug/vachat-server
```

调试版本适合本地开发验证，体积较大，运行性能也低于 release 版本。

### 3. 编译发布版本

```bash
cargo build --release
```

编译产物位于：

```text
target/release/vachat-server
```

发布部署时，至少需要准备以下目录和文件：

```text
vachat-server
config/
wwwroot/
data/
```

其中：

- `vachat-server`：编译得到的服务端可执行文件。
- `config/`：服务端配置目录，包含 `config.toml` 等配置文件。
- `wwwroot/`：Web 前端静态资源目录。
- `data/`：运行数据目录，首次启动时可以为空。

启动示例：

```bash
./target/release/vachat-server \
  --network.bind 0.0.0.0:3000 \
  --network.domain localhost
```

启动后访问：

```text
http://localhost:3000
```

### 4. 编译 Linux 静态二进制

如果希望构建更适合 Docker 或服务器部署的 Linux 静态二进制，可以使用项目中的 Docker 编译方式。该方式依赖本机已安装 Docker。

```bash
sudo apt install -y docker.io
sudo usermod -aG docker $USER
```

重新登录终端后确认 Docker 可用：

```bash
docker version
```

项目中的构建脚本位于：

```text
build/docker/build.sh
```

脚本会使用 `clux/muslrust:stable` 编译 `x86_64-unknown-linux-musl` 版本，并把产物复制到 `build/docker/vachat-server`。

需要注意：当前脚本后半段包含镜像 tag 和 push 操作。如果只是本地编译，建议参考脚本中的核心命令手动执行：

```bash
docker run --rm -it \
  -v "$(pwd)":/home/rust/src \
  -v "$(pwd)/config/cargo.toml":/root/.cargo/config.toml \
  -w /home/rust/src \
  clux/muslrust:stable cargo build --release
```

编译完成后产物位于：

```text
target/x86_64-unknown-linux-musl/release/vachat-server
```

### 5. 构建 Docker 镜像

先准备 Docker 构建目录需要的文件：

```bash
cp target/x86_64-unknown-linux-musl/release/vachat-server build/docker/vachat-server
cp -r config build/docker/config
```

然后构建镜像：

```bash
cd build/docker
docker build --platform=linux/amd64 -t vachat-server:latest .
```

运行镜像：

```bash
docker run -d --restart=always \
  -p 3000:3000 \
  --name vachat-server \
  -v ./data:/home/vachat-server/data \
  vachat-server:latest
```

也可以参考：

```text
build/docker/docker-compose.yaml
```

## vachat-web 浏览器端

当前 `vachat-server` 仓库没有包含 Web 前端源码，只包含已经构建好的静态资源目录：

```text
wwwroot/
```

服务端启动时会通过配置项读取该目录：

```toml
[system]
wwwroot_dir = "./wwwroot"
```

如果只是编译和部署服务端，不需要重新构建 Web，保留仓库中的 `wwwroot/` 即可。

如果需要重新编译 Web 前端，通常流程如下：

### 1. 安装 Node.js 环境

推荐使用 Node.js LTS 版本。安装完成后确认版本：

```bash
node -v
npm -v
```

如果前端项目使用 pnpm，安装 pnpm：

```bash
npm install -g pnpm
pnpm -v
```

### 2. 获取 Web 前端源码

进入 Web 前端源码目录，例如：

```bash
cd vachat-web
```

安装依赖：

```bash
npm install
```

或：

```bash
pnpm install
```

### 3. 配置服务端地址

根据 Web 项目的实际配置方式，设置 API 地址、站点域名等环境变量。常见文件名包括：

```text
.env
.env.production
```

如果 Web 前端和服务端部署在同一域名下，通常可以使用相对路径访问 API，不需要额外配置跨域。

### 4. 构建 Web 静态资源

常见构建命令为：

```bash
npm run build
```

或：

```bash
pnpm build
```

构建产物通常位于：

```text
dist/
build/
```

具体目录以 Web 项目的构建配置为准。

### 5. 拷贝产物到服务端

将 Web 构建产物复制到 `vachat-server` 的 `wwwroot/`：

```bash
rm -rf /path/to/vachat-server/wwwroot/*
cp -r dist/* /path/to/vachat-server/wwwroot/
```

然后重新启动 `vachat-server`。访问服务端地址时，就会加载新的 Web 页面。

## vachat-app 移动应用

`vachat-app` 是 Flutter 项目，项目支持 Android 和 iOS。编译前建议先准备一个可访问的 `vachat-server` 服务端地址，例如：

```text
https://chat.example.com
```

如果要支持相机、文件上传、语音、WebSocket、系统分享等移动端能力，建议使用正式域名和有效 TLS 证书。

### 1. 安装 Flutter 环境

安装 Flutter SDK 后，将 `flutter/bin` 加入 `PATH`，然后检查环境：

```bash
flutter --version
flutter doctor
```

本项目 `pubspec.yaml` 中要求 Dart SDK 版本：

```text
>=3.11.0 <4.0.0
```

如果 `flutter doctor` 提示 Android、Xcode 或 CocoaPods 缺失，按提示补齐对应平台环境即可。

进入移动端源码目录并安装依赖：

```bash
cd D:/workspace/vachat/vachat-app
flutter pub get
```

如果修改过 JSON 模型或需要重新生成代码，可以执行：

```bash
dart run build_runner build --delete-conflicting-outputs
```

如果修改过应用图标或启动页资源，可以执行：

```bash
dart run flutter_launcher_icons
dart run flutter_native_splash:create
```

### 安卓应用

#### 1. 安装 Android 开发环境

安装 Android Studio，并在 SDK Manager 中安装：

- Android SDK Platform
- Android SDK Build-Tools
- Android SDK Platform-Tools
- Android Emulator，可选
- Android NDK `28.2.13676358`

项目 Android 配置使用 Java 17，确认本机 JDK 版本：

```bash
java -version
```

配置 Android SDK 环境变量：

```bash
export ANDROID_HOME="$HOME/Android/Sdk"
export PATH="$ANDROID_HOME/platform-tools:$ANDROID_HOME/tools:$ANDROID_HOME/tools/bin:$PATH"
```

确认 Android 工具可用：

```bash
adb version
flutter doctor --android-licenses
flutter doctor
```

#### 2. 配置 Android 签名

调试包不需要配置发布签名；如果要编译 release APK，需要先创建 keystore。

在 `vachat-app/android` 目录执行：

```bash
keytool -genkey -v \
  -keystore app/vachat-release.jks \
  -keyalg RSA \
  -keysize 2048 \
  -validity 10000 \
  -alias vachat \
  -storepass xxxx \
  -keypass yyyy \
  -dname "CN=vachat, OU=vachat, O=vachat, L=Beijing, ST=Beijing, C=CN"
```

然后创建 `vachat-app/android/key.properties`：

```ini
storePassword=xxxx
keyPassword=yyyy
keyAlias=vachat
storeFile=vachat-release.jks
```

`storeFile` 是相对 `android/app/` 的路径；上面的命令会把证书生成到 `android/app/vachat-release.jks`。

#### 3. 编译 Android 调试包

```bash
cd D:/workspace/vachat/vachat-app
flutter build apk --debug
```

调试包产物通常位于：

```text
build/app/outputs/flutter-apk/app-debug.apk
```

#### 4. 编译 Android 发布包

编译通用 release APK：

```bash
flutter build apk --release
```

产物通常位于：

```text
build/app/outputs/flutter-apk/app-release.apk
```

如果需要按 CPU 架构拆分 APK，减少单个安装包体积：

```bash
flutter build apk --release --split-per-abi
```

会生成类似以下文件：

```text
build/app/outputs/flutter-apk/app-arm64-v8a-release.apk
build/app/outputs/flutter-apk/app-armeabi-v7a-release.apk
build/app/outputs/flutter-apk/app-x86_64-release.apk
```

如果只构建某个架构：

```bash
flutter build apk --release --target-platform android-arm64
flutter build apk --release --target-platform android-arm
flutter build apk --release --target-platform android-x64
```

如果需要上架 Google Play，建议构建 AAB：

```bash
flutter build appbundle --release
```

产物通常位于：

```text
build/app/outputs/bundle/release/app-release.aab
```

### IOS应用

iOS 应用需要在 macOS 上编译，并安装 Xcode。

#### 1. 安装 iOS 开发环境

安装 Xcode 后执行：

```bash
sudo xcode-select -s /Applications/Xcode.app/Contents/Developer
sudo xcodebuild -license accept
```

安装 CocoaPods：

```bash
sudo gem install cocoapods
pod --version
```

检查 Flutter iOS 环境：

```bash
flutter doctor
```

#### 2. 安装 iOS 依赖

进入移动端源码目录：

```bash
cd D:/workspace/vachat/vachat-app
flutter pub get
```

在 macOS 上进入 iOS 工程安装 Pods：

```bash
cd ios
pod install
cd ..
```

如果仓库中暂时没有 `ios/Podfile`，可以先执行一次 Flutter iOS 构建或运行命令，Flutter 会按项目配置生成所需的 iOS 辅助文件。

#### 3. 配置签名

打开 Xcode workspace：

```bash
open ios/*.xcworkspace
```

在 Xcode 中配置：

- Bundle Identifier
- Team
- Signing Certificate
- Provisioning Profile

iOS 真机调试、TestFlight 和正式分发都依赖 Apple Developer 账号。

#### 4. 编译 iOS 调试版本

连接 iPhone 或启动模拟器后执行：

```bash
flutter run
```

也可以在 Xcode 中选择设备后点击 Run。

#### 5. 编译 iOS 发布版本

构建 IPA：

```bash
flutter build ipa --release
```

如果需要通过 Xcode 归档，可以选择：

```text
Product → Archive
```

归档完成后，在 Organizer 中导出 IPA 或上传到 App Store Connect。
