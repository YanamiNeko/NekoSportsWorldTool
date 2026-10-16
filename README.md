<h1 style="font-size: 48px;">别倒卖了😭</h1>

# NekoSportsWorldTool

![Release](https://img.shields.io/github/v/release/YanamiNeko/NekoSportsWorldTool)
![Build](https://github.com/YanamiNeko/NekoSportsWorldTool/actions/workflows/release.yml/badge.svg)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue)

运动世界校园版全协议自动化工具，纯 Rust 单文件，GUI / CLI 双模式。

> ⚠️ 仅供学习研究，请勿用于违反学校规定的场景，后果自负。

## 功能

- **跑步**：一键全链（策略 → 打卡点 → 轨迹 → 提交 → OBS → 验证），轨迹按打卡点自然生成，开始时间支持随机或指定到前 3 天
- **AI 运动**：任务 / 自由练习双模式，批量补签（前 60 天 × 多项目）
- **数据**：学期完成度、违规自查、排行榜（个人/班级/院系 × 日/月、室内榜、历史榜）、个人主页
- **登录**：支持人机验证自动通过
- **自动更新**：桌面端检查/下载/自替换并重启（GitHub Release 直连，无镜像）；Android 下载 APK 调起系统安装器；启动检查模式可选（静默/询问/关闭）

## 界面

![GUI](docs/screenshot.png)

## 技术特点

- 完整实现 NetSecKit 信封加密与响应解密，每个响应强制 RSA 验签（fail-closed）
- 人机验证全程本地完成，不依赖任何外部服务
- 轨迹生成器：打卡点拟合环 + 自然速度曲线 + 哨兵点 / 断崖 / 点位吸附
- 设备身份持久化（DeviceId / 安装时间 / MAC），稳定不易触发风控
- GUI 与 CLI 共用同一套协议层，数据文件互通

## 使用

双击 exe 进入 GUI；CLI 用法：

```text
NekoSportsWorldTool login --user <手机号> --pass <密码> --remember
NekoSportsWorldTool run                                  # 一键跑步（--altitude 17.2 固定海拔，或 11.6-22.8 映射到区间）
NekoSportsWorldTool ai --sport 2 --score 26000           # AI 运动
NekoSportsWorldTool rank main --type 1 --sort 1          # 排行榜
NekoSportsWorldTool update [--check]           # 自动更新（--check 仅检查不下载）
NekoSportsWorldTool template --file run.gpx   # 本地分析真实记录海拔（不会上传）
NekoSportsWorldTool help                                 # 全部命令
```

`template` 只读取用户手动选择的本地 GPX/JSON 文件，输出采样点、海拔范围、累计上升和累计下降，
不会登录、访问服务器或把模板记录接入跑步上传流程。

手动海拔支持三种状态：留空使用自动海拔；填写单个数字（例如 `17.2`）让所有轨迹点使用固定海拔；填写区间（例如 `11.6-22.8`）将生成的海拔曲线映射到该上下限内，同时保留轨迹的相对起伏。

GUI 的设备设置支持「在线地图选点」：可切换 OpenStreetMap 街道图与 Esri 卫星影像，点击选择经纬度，拖动平移，滚轮、双指手势或按钮缩放，并按 `0.000001°` 微调。坐标使用 WGS84；小数位数表示输入精度，实际准确性取决于底图。点击「保存」后坐标才生效；地图加载失败时仍可手动输入。地图仅在打开选点时联网，瓦片缓存在数据目录的 `map_tiles/` 中。

获取系统定位或改变坐标后，会自动识别中国城市：首次从阿里云 DataV 下载约 4 MB 全国城市行政边界，保存为 `city_boundaries.json`，之后在本地按坐标匹配城市，不向城市查询服务发送坐标。旧默认城市与新位置不一致时会重新识别；失败可手动填写，识别过程中手动填写的城市不会被覆盖。中国范围外、城市边界附近仍需手动核对。

联网范围：城市识别只下载全国数据，在本地匹配；在线地图会向底图服务商请求当前视野的瓦片，瓦片编号可推知查看区域。「查找附近学校/操场」和「在线下载附近路网」会向 Overpass 发送所选中心坐标及查询半径。无需联网查看位置时，可使用手动输入和本地 OSM 导入。

网络失败与重试：下载均设有超时。Overpass 返回 HTTP 429、503 或 504 时会显示限流/繁忙提示；若返回秒数形式的 `Retry-After`，提示至少等待相应时间再手动重试。不自动循环重试。路网缓存保存失败会显示错误，当前已加载路网保持不变；请检查数据目录写入权限和可用空间后重试。

操场选点：打开在线地图，点击「查找附近学校/操场（OSM）」查询地图中心附近 2 公里的学校与运动设施，点击列表的「查看」定位并高亮轮廓，再切换「卫星（Esri）」放大到实际椭圆跑道后点击选点并保存。OSM 可能缺少校园标注，卫星影像也可能有时效或偏移；未找到列表结果时仍可在影像上手动选点。学校没有单独标注操场时，先点击校园的「查看」，再在卫星图上找到实际跑道。校园与设施中心用于查看，不会自动作为你的实际位置。

路网页支持「在线下载附近路网」：使用已保存锚点或地图选点作为中心，下载半径 200–2000 米内的道路与建筑，包括标注了 `leisure=track` 和 `sport=running/athletics` 的跑道线；面状运动场不会被转换成道路。下载由 Overpass API 提供，成功后接入现有路网预览和路线规划，缓存到 `online_roads/`，同一区域 7 天内复用缓存，重启自动恢复已下载路网。下载失败保留当前路网。同时支持本地 OSM XML 文件导入：填写路径，或在桌面端浏览、拖入 `.osm` / `.xml` 文件。兼容旧 `osm_path` 配置，重启恢复最近成功加载的来源；导入失败保留当前路网。Android 仅支持应用可访问的文件路径，尚无系统文件选择器。

Android 的「读取本机信息和定位」在确认读取设备信息后请求系统位置权限，获取一次当前位置并显示精度。仅获取前台位置，不持续跟踪；拒绝权限、位置服务关闭或超时不影响设备信息读取。坐标只填入编辑区，需核对城市并点击「保存」。

Windows、Linux 和 macOS GUI 的设备页支持「获取本机系统定位」：最多等待 60 秒，可取消；成功显示定位精度、填写坐标并自动识别城市，仍需手动保存。电脑定位可能较粗，请用卫星图校准操场。拒绝权限、定位失败或取消不会改动原坐标。

- Windows 使用 WinRT；需在系统的「隐私和安全性 → 位置」允许位置服务及桌面应用访问。
- macOS 使用 Core Location；请启动发布包中的 `NekoSportsWorldTool.app`，按系统提示授权。应用包包含定位用途说明；裸二进制不能申请定位权限。
- Linux GUI 使用系统 D-Bus 的 GeoClue2，需安装并启用 GeoClue2、允许桌面位置访问，且桌面会话需要支持定位授权。GUI 包附带 `org.nekosportsworld.tool.desktop`；将程序放入 PATH，并把该文件安装至 `~/.local/share/applications/` 后从应用启动器打开。缺少服务或授权时仍可使用地图选点。Linux CLI 包继续独立提供。

青龙定时任务：首次手工 `login --remember` 一次，之后定时挂 `run` 即可。

## 常见问题

- **提示未登录 / 401**：先执行 `login --remember`（GUI 勾选记住密码等价），会话失效会自动重登
- **10121 设备风险**：设备身份是持久化的，删除 `identity.json` 等于换了新设备，不要频繁删
- **10603 点位限流**：点位接口 5 分钟限 3 次，程序内置 300s 缓存，正常使用不会触发
- **榜单为空**：当天还没有人产生有效里程，查询会自动回退最近 3 天
- **检查更新失败**：更新仅直连 GitHub，网络不通时到 [Release](https://github.com/YanamiNeko/NekoSportsWorldTool/releases) 手动下载
- **手动更新方式**：Windows 下载新的 `nekosportsworldtool.exe` 后退出软件，用新文件替换旧文件；请保留 `config.json`、`identity.json`、`points_cache.json` 等配置文件，不要删除或修改。
- **人机验证失败**：自动重试 3 轮，仍失败大概率是网络波动，稍后再试
- **杀软误报**：未签名编译产物可能被误报，自行判断后加白

## 构建

```bash
cargo build --release                        # GUI + CLI
cargo build --release --no-default-features  # 仅 CLI
```

推 tag 自动构建 Windows / macOS / Linux 四平台产物并发布 Release（见 `.github/workflows/release.yml`）。

## 许可证

[CC BY-NC 4.0](LICENSE) © YanamiNeko（禁止商用）

### 登录信息 ZIP 备份

登录栏提供「导出登录 ZIP」和「导入登录 ZIP」。Android 使用系统文件保存/选择窗口，无需 Root；桌面版使用文件对话框，两个平台共用备份格式。

- ZIP 内的 `login.json` 包含格式版本、完整会话（uid/token/unid/profile）、设备身份以及已保存的账号、记住密码开关和密码；未记住的密码不导出。
- 导入前确认替换当前账号和设备身份；跑步参数、路网设置保留。导入后刷新账号页面，凭证是否仍有效取决于服务器。
- ZIP 未加密，请妥善保管，不要发给他人或提交到仓库。导出需要本地已有登录会话；后台任务结束后才能操作。
- 导入仅接受本工具生成的格式，限制 4 MB，校验版本和设备身份，拒绝额外文件、路径穿越和损坏数据；校验失败不会修改原登录信息。
