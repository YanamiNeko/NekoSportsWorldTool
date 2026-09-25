# Android APK

Android 移植保留 Rust 业务逻辑、桌面 GUI/CLI 与七个功能页面。手机端沿用原界面配色，增加竖屏排版、触控数值输入和内置中文字体；支持应用内自更新（关于页检查，下载 APK 后调起系统安装器）。

## 使用方式

- 面向 Android 16（target SDK 36）的 ARM64 手机；声明最低 API 26，旧系统未进行设备验收。
- 安装 `build/arm64-v8a-release/NekoSportsWorldTool-arm64-v8a.apk`。这是本地测试签名包，允许手动安装后即可打开。
- 点击账号、密码或数值字段，会弹出 Android 原生编辑框，使用手机当前默认输入法；“确定”或键盘完成键回填，“取消”保留原值。
- 首次启动询问是否读取本机品牌、型号和系统版本：同意后保存，拒绝后仍可手填。设备页可再次询问并读取，手动读取需点击“保存”才生效。
- 品牌作为独立字段保存在 `identity.json`，重启后可在设备页查看和编辑，仅供本地展示；服务端请求保持原有机型字段。旧版配置缺少品牌时按空值加载，可再次读取并保存，已有 UUID 不变。
- DeviceId 沿用原项目首次生成并持久化的 UUID；读取本机信息不更换 UUID，也不改动手填 IMEI、MAC、安装时间或位置。读取不会申请电话权限。
- 登录、跑步与 AI 任务执行时保持屏幕常亮；任务结束或切到后台后解除。使用期间请让 App 保持前台，后台或锁屏运行不作保证。
- 数据保存在 Android 应用私有 `files` 目录；卸载/清除应用数据会删除它们。APK 不带电脑端的账号、会话和配置。
- 自更新：默认启动静默检查（可在关于页改为询问/关闭），发现新版本下载 APK 后调起系统安装器；首次安装需在系统设置允许本应用"安装未知应用"。检查上游与 fork 两个仓库，取带 APK 资产且版本最新的 Release。

## 发布（自更新下载源）

推 APK 到 GitHub Release 用 `android/release.ps1`（需要 gh CLI 已登录）：

```powershell
./android/release.ps1 -AndroidRev 4                       # 发到 fork（默认）
./android/release.ps1 -AndroidRev 5 -Repo YanamiNeko/NekoSportsWorldTool -Notes "..."
```

脚本会读 Cargo.toml 版本、盖章 AndroidManifest（versionName=`<版本>-android.<N>`）、构建 arm64 并以固定资产名 `NekoSportsWorldTool-android-arm64.apk` 上传。**必须一直用同一个签名密钥**（`android/.signing/local-test.keystore`）：签名变化后手机无法覆盖安装，卸载重装还会丢 identity/session。

## CI 自动构建（`.github/workflows/apk.yml`）

推送 main 或 tag（`v*`）时在 Ubuntu runner 上自动构建 arm64 APK，流程为本目录 `build.ps1` 的 Linux 移植（NDK 29 交叉编译、ELF 16KB 对齐检查、javac/d8/aapt2/zipalign/apksigner 校验）：

- **main / 手动触发**：仅产出 Actions 构建产物（不发布），用于验证 Android 链路可编译；无签名 Secret 时用一次性调试密钥，仅供冒烟安装。
- **tag 触发**（如 `v0.2.6` 或 `v0.2.6-android.2`）：自动盖章版本（以 tag 为准，`<版本>-android.<rev>`，versionCode 编码与 release.ps1 相同），并把 `NekoSportsWorldTool-android-arm64.apk` 挂到该 tag 的 Release——Android 端自更新按此资产名精确匹配。

**发布前必须配置仓库 Secret `ANDROID_KEYSTORE_BASE64`**（签名 keystore 文件的 base64，建议直接用 `android/.signing/local-test.keystore` 以保持与历史 APK 签名连续；可选 `ANDROID_KEYSTORE_PASSWORD` / `ANDROID_KEY_ALIAS` / `ANDROID_KEY_PASSWORD`，默认 `android` / `androiddebugkey` / `android`）。tag 构建缺失该 Secret 会直接失败——换密钥静默发布会让所有已装用户无法覆盖安装。生成 base64：

```bash
base64 -w0 android/.signing/local-test.keystore   # Windows: certutil -encode ... 后去掉头尾换行
```

## Windows 构建

需要 PowerShell 7、Rust、JDK 21（`javac` 在 PATH 中）和以下 Android SDK 组件：

```text
platforms;android-36
build-tools;36.0.0
ndk;29.0.14206865
platform-tools
```

在项目根目录执行：

```powershell
rustup target add aarch64-linux-android
./android/build.ps1 -Abi arm64-v8a -Profile release
```

SDK 默认使用 `ANDROID_HOME`，未设置时使用 `%LOCALAPPDATA%\Android\Sdk`；可通过 `-SdkPath` 覆盖。编译和打包中间文件放在 `%TEMP%\neko-android-<项目路径摘要>`，避免 NDK/aapt2 的中文路径问题；可通过 `-TargetDir` 指定其他纯 ASCII 缓存目录。

脚本执行 Rust 编译、Java/Dex 编译、资源打包、ELF 16 KB 对齐检查、zipalign 和 APK 签名验证。最终 APK 放回 `android/build/`。

首次构建生成 `android/.signing/local-test.keystore`，仅用于本地测试，不是应用商店发布密钥。该文件和构建产物已被 Git 忽略；保留密钥才能覆盖安装此前由它签名的版本。

## 离线验证

```powershell
cargo test --lib --locked
cargo check --locked --all-targets
rustup target add x86_64-linux-android
./android/build.ps1 -Abi x86_64 -Profile debug
# 先启动 Android 16 x86_64 模拟器，用 adb devices 查看实际序列号：
$serial = 'emulator-5554' # 替换为你的模拟器序列号
adb -s $serial install -r ./android/build/x86_64-debug/NekoSportsWorldTool-x86_64.apk
./android/test.ps1 -Serial $serial
```

`test.ps1` 仅接受显式的模拟器序列号；它重置模拟器内测试应用的设备身份和读取选择，测试同意/拒绝、UUID 复用、私有目录、任务常亮开关、默认输入法连接、中文/表情组合输入、密码遮罩、数值键盘，以及 Activity 退出、重开与重建。测试不登录也不提交业务数据。

本次验证：42 项 Rust 离线测试通过；Android 16 x86_64 模拟器的 45 项集成检查和 4 项独立进程重启持久化检查通过，包括品牌保存/恢复、UUID 复用和授权后立即退出的恢复，并检查七页面、360/412dp 排版及文本/数值实际回填。实际 ARM64 手机、账号登录和服务器业务流程仍需在用户设备验收；模拟器使用 4 KB 页面，16 KB 兼容性仅做了 ELF/打包静态检查。

## 平台实现

- `src/android.rs` 和 `MainActivity.java` 提供 NativeActivity 入口、JNI 编辑桥、剪贴板和屏幕策略。
- Android TLS 使用 ureq 的 WebPKI 根证书校验；没有关闭证书验证。
- `vendor/winit-0.30.13/NEKO-PATCH.md` 记录 Android 销毁/重建事件循环及缩放变化检测补丁；桌面路径不变。
- 中文字体为 Noto Sans SC，来源与 OFL 许可证在 `assets/fonts/`。

应用仅声明联网权限，没有后台服务、唤醒锁或存储访问权限。
