# 2026-10-24 原始详情与 BUG 修复合并版

- 合并 codex/buglist-20261024，并保存、整合主目录原有详情改动。
- GUI 保留原始 speed/statusInfo/reasonList；CLI `record --rrid <id>` 的 stdout 只输出详情 JSON，诊断走 stderr。
- 重复参数、额外参数和非法编号报错；服务端返回错误记录编号不展示。
- 修复学期距离误作次数、count=0 误报违规、移动时间窗口零步及默认道路模式。
- Windows 构建链为 x86_64-pc-windows-gnu；版本由 build.rs 注入提交标识。ZIP 仅包含四个白名单文件。

最终源码提交、测试通过数、EXE/ZIP SHA256 与 --version 输出见同名 Release 的发布说明和测试报告附件。报告在合并后的完整测试与 Windows 构建完成后生成，不复用旧版校验值。

P0-2 和 P1-10 待抓包验证，不猜单位/结构；App 最快配速的来源、真实地形海拔仍待验证。未使用真实成绩提交或 OBS 上传测试，Android 未验证。保留既有编译警告，地图服务可用性与地图覆盖不能由离线测试保证。
