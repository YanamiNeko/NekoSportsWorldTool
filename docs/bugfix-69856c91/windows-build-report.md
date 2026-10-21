# Windows 构建验证（2026-10-20）

交付：NekoSportsWorldTool-Windows-x64-road-loop.exe，版本 0.3.0，Windows x64，包含 GUI 和 CLI，未签名。

Ubuntu WSL 的 Rust 工具链交叉编译至 x86_64-pc-windows-gnu，默认 GUI 开启，静态链接运行库。命令：

```
cargo build --release --locked --offline --target x86_64-pc-windows-gnu --bin nekosportsworldtool
cargo test --release --locked --offline --target x86_64-pc-windows-gnu --lib --no-run
```

Windows 本机 163 项库测试通过；help 退出码 0；隔离无账号目录启动主窗口，8 秒后仍响应。EXE 的 SHA-256 与实际测试文件一致。未进行真实成绩提交，未验证其他 Windows 版本或 Android。完整范围见 route-refresh-verification.md。
