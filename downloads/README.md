# Windows 道路闭环版（2026-10-20）

[下载 Windows x64 压缩包](https://github.com/GAN1112233131/NekoSportsWorldTool/raw/refs/heads/fix/confirmed-bugs-69856c91/downloads/NekoSportsWorldTool-Windows-x64-road-loop.zip)

解压后先关闭旧程序，将新版 EXE 放入原工具目录运行，保留原配置和地图。版本号仍为 0.3.0，校验值见 SHA256SUMS.txt。

预览和正式道路生成均支持无围栏；额外里程沿已覆盖点位的同一道路闭环重复。有有效围栏时仍应用其范围，无围栏无法保证全程在校内。

GUI 178 项、无 GUI 122 项、Windows 本机 163 项测试通过，Windows 窗口启动检查通过。手动 OSM 和自动获取均完成真实点位的只读预览验收；未进行真实成绩提交或 OBS 上传。

[完整验证报告](../docs/bugfix-69856c91/route-refresh-verification.md) · [源码修改说明](../BUGFIX_NOTES.md)

本文件记录本次 GitHub 上传；验证报告中的“未推送”是构建验收时状态。本次只更新修复分支并提供下载文件，没有发布 Release、创建 PR 或关闭议题。包内不含账号配置、密码、会话或设备身份。
