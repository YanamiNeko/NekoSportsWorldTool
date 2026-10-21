# UI 改版：青绿主题 · 卡片式布局（Windows + Android）

## 概述

本 PR 按设计稿（见下方截图）重做了整个 UI 层，**业务逻辑零改动**。桌面端与安卓端共用同一套 `eframe/egui` 代码，两端同步生效。

## 改了什么（仅 UI）

- `src/ui/theme.rs`：设计 token —— 青绿主色阶、5 级字号、4pt 间距栅格、圆角分层、卡片阴影。
- `src/ui/components.rs`（新增）：共用组件 `card` / `chip` / `segmented` / `notice` / `stat`。
- `src/ui/mobile.rs`：桌面端分组侧栏、手机端底部 5 项导航、二级页返回条；响应式断点（手机 / 平板 / 桌面）。
- `src/ui/mod.rs`：登录改为弹窗，顶栏改为状态胶囊，应用骨架重排。
- 9 个页面全部卡片化：跑步、AI 运动、记录、数据、我的、设备信息、运行日志、路网（地图）、关于。

## 复审后修复

- **安卓导航**：设备信息 / 运行日志 / 路网 / 关于与更新等工具页，从「我的」页的「更多设置」进入二级页，顶部带「← 我的」返回条。9 个页面仍然全部可达，与改版前一致 —— 只改 UI 形态，不动逻辑。
- **安卓模糊**：部分机型通过 `AConfiguration` 上报的默认像素密度为 160，导致 egui 按 1 倍渲染、整体发虚。现在由 Activity 上报权威的 `DisplayMetrics.density`（配置变化时刷新），内嵌的 winit 在 `scale_factor()` 中优先采用它。
  涉及文件：`src/android.rs`、`android/java/org/nekosportsworld/tool/MainActivity.java`、`vendor/winit-0.30.13/src/platform_impl/android/mod.rs`。
- **窄屏顶栏**：状态胶囊与登录按钮不再在窄屏下挤成竖排，手机上顶栏拆为两行。

## 未改动（已核对）

- 协议、API、加密、轨迹、更新等逻辑均未触碰。
- `src/ui/osm.rs`、`src/ui/map.rs`、`crates/route-planner` 均为仓库原有实现 —— 本 PR 只换外观，不新增路由 / 路网功能。
- `cargo test --features gui --lib`：184 passed / 0 failed。

## 已解决与 `main` 的冲突

已合入最新 `main`（含 PR #88），合并提交 `ed90ce7`，冲突清零（`mergeable_state` 已从 `dirty` 变为 `clean`）：

- 上游新增的 `crates/route-planner`（boundary / check / gpx）、`api/school_roads`、`api/submission_guard`、`ui/road_tools`、`ui/layout_tests` 等逻辑**全部保留**；
- `src/ui/{data,mod,osm,run,user}.rs` 共 16 处冲突逐个手工合并，原则是**以上游逻辑为准，只叠加卡片外观**。

## 截图

### Windows（桌面，1218×764）

| 跑步 | AI 教练 |
|---|---|
| ![run](https://raw.githubusercontent.com/kek2233/NekoSportsWorldTool/assets/ui-screenshots/win_run.png) | ![ai](https://raw.githubusercontent.com/kek2233/NekoSportsWorldTool/assets/ui-screenshots/win_ai.png) |

### Android（手机布局，378×844）

| 首页（底部 5 栏） | 我的（更多设置入口） | 设备信息二级页 |
|---|---|---|
| ![phone home](https://raw.githubusercontent.com/kek2233/NekoSportsWorldTool/assets/ui-screenshots/phone_home.png) | ![phone user](https://raw.githubusercontent.com/kek2233/NekoSportsWorldTool/assets/ui-screenshots/phone_user.png) | ![phone device](https://raw.githubusercontent.com/kek2233/NekoSportsWorldTool/assets/ui-screenshots/phone_device.png) |

### 设计稿（`docs/ui-redesign/preview.html`，按 `.gitignore` 不入库）

| 桌面 | Android |
|---|---|
| ![desktop mockup](https://raw.githubusercontent.com/kek2233/NekoSportsWorldTool/assets/ui-screenshots/preview_desktop.png) | ![android mockup](https://raw.githubusercontent.com/kek2233/NekoSportsWorldTool/assets/ui-screenshots/preview_android.png) |

## 测试计划

- [x] Windows：运行 `nekosportsworldtool.exe`，确认 9 个页面均以青绿主题正常渲染。
- [x] Android：安装 arm64 apk，在手机上验证底部导航与卡片布局。
- [x] 确认登录弹窗、状态胶囊、侧栏 / 底部导航的功能行为与改版前一致。
- [x] Android 上 9 个页面全部可达（工具页位于「我的」→「更多设置」）。
