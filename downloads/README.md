# Windows 配速与提交保护修复版（2026-10-20）

[下载最新版 Windows x64 压缩包](https://github.com/GAN1112233131/NekoSportsWorldTool/releases/download/windows-pace-fix-2026-10-20/NekoSportsWorldTool-Windows-x64-pace-fix.zip)

首次使用可解压运行 `Windows配速修正版/NekoSportsWorldTool-Windows-x64-pace-fix.exe`。已有工具先关闭旧程序，只复制新版 EXE 到原工具目录，保留原配置、登录状态、地图及 `submission_receipts/`。不要用包内空配置覆盖原配置；独立目录之间不会共享重复提交保护。版本号仍为 0.3.0，包内自动更新默认关闭。

本版修正 OBS 每公里配速单位，增加准确的违规状态、关联详情和重复提交保护，继承道路刷新及闭环重复修复。无围栏无法保证全程在校内，旧服务器记录不会自动更新。

GUI 197 项、无 GUI 136 项、Windows 本机 182 项测试通过，编译与 Windows 启动检查通过。未提交真实成绩或上传 OBS；官方 App 最终显示、Android 尚未实测。

[验证报告](../docs/integration-verification.md) · [配速说明](../docs/pace-unit-fix.md) · [重复提交保护](../docs/violation-checks.md) · [本版 SHA-256](SHA256SUMS-pace-fix.txt)

本目录中的 `NekoSportsWorldTool-Windows-x64-road-loop.zip` 及 `SHA256SUMS.txt` 保留为历史道路闭环版，不包含本次新增修复。最新版仅在上面的 Release 链接提供 ZIP；包内包含空配置和使用说明，不含用户账号、密码、会话或设备身份。
