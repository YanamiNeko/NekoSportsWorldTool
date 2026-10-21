# 逐项议题处理表

基线 69856c91。状态仅描述本补丁与离线验证，不表示已关闭议题或官方 App 全面修复。

二次复查补充：#28/#48 的围栏坐标与区域 ID、#47 的异常点后处理、#76 的分钟距离取整、#83 的点位缓存隔离和策略点完整性已修正，详见 review-report.md。原有平台与服务端验证边界仍保留。

| 议题 | 状态 | 本次处理 | 尚未验证/边界 |
|---|---|---|---|
| [#28 关于最新版本的一些bug](https://github.com/YanamiNeko/NekoSportsWorldTool/issues/28) | 部分修复 | 共享围栏及点位包装；实际判定 isPass；补累计 steps；统计来自最终轨迹；减少异常漂移。 | 轨迹颜色、围栏及勾选的客户端显示未实测。累计爬升不等于首尾海拔差，保留正差分累计语义。 |
| [#47 有个问题](https://github.com/YanamiNeko/NekoSportsWorldTool/issues/47) | 部分修复 | 经典和道路模式额外异常位移限制为 6 米，并限制半径和 GPS 速度。 | 灰色/红色含义及客户端呈现需官方版本和记录样本。 |
| [#48 轨迹按照打卡点生成问题](https://github.com/YanamiNeko/NekoSportsWorldTool/issues/48) | 范围内修复 | 道路模式使用全部路线点，不再静默回退；不可达明确报错；保留折返点。经典模式显示建筑数据限制。 | 经典模式仍可能横穿建筑；道路模式依赖 OSM 完整性，不承诺避开所有建筑。 |
| [#50 为什么这个软件刷出来的没有点位数据？](https://github.com/YanamiNeko/NekoSportsWorldTool/issues/50) | 部分修复 | 提交与 OBS 复用同一份点位结果，保留普通/必经分类与原 ID。 | 只确认数据完整性；官方 App 点位展示未实测。 |
| [#59 运动达标但是没法计入成绩，排行榜上也有](https://github.com/YanamiNeko/NekoSportsWorldTool/issues/59) | 提示修复 | 详情读取、OBS 上传、记录提交及服务端判定分开展示；缺判定为未知。 | 不能通过读取成功或上榜推断入账，服务端学期计分规则未变更。 |
| [#62 vivo小屏手机屏幕占比有问题，导致部分功能使用不了。](https://github.com/YanamiNeko/NekoSportsWorldTool/issues/62) | 桌面离线 UI 验证通过 | 紧凑账号折叠、压缩状态栏、数据单一纵向滚动；短屏和桌面点击/滚动回归。 | Android 构建与 smoke 受工具链缺失阻塞，vivo 真机触摸/键盘尚未验证。 |
| [#76 具体运动数据偏差大](https://github.com/YanamiNeko/NekoSportsWorldTool/issues/76) | 部分修复 | 最终轨迹统一统计、真实尾窗、公里边界；分钟/公里和厘米固定数值测试；零步数无虚假巨大步幅。 | 官方 App 的 0.06 配速及万厘米展示需版本、完整脱敏记录；不标为全部解决。 |
| [#83 且目前最新版本的经典卡卡使用缺失了常规点位，切了路网模式，且轨迹不在点位上仍能打卡成功](https://github.com/YanamiNeko/NekoSportsWorldTool/issues/83) | 源码缺陷修复 | 全部策略/实时点整理去重；普通点保持分类；漏点 isPass=false；道路轨迹漏点直接报错。 | 上传成功与服务端达标分开；服务器实际判定需服务端证据。 |
| [#85 上传成功了但是没有点位，有什么影响吗？](https://github.com/YanamiNeko/NekoSportsWorldTool/issues/85) | 部分修复 | 点位包装统一，OBS 部分失败可见，结果提示不把详情可读当成达标。 | 官方展示和成绩影响取决于服务端及 App 版本，未进行真实提交验证。 |
| [#64 我设置的是规定的时间但却不能用](https://github.com/YanamiNeko/NekoSportsWorldTool/issues/64) | 待补证据 | 协议日期与 OBS 目录统一北京时间；保留现有 UI 时间模式。 | 时间限制根因未确定，需要对应版本、完整错误及脱敏策略数据。 |


学校路网后续需求已另行实现并回归，详见 school-road-report.md；此前议题的服务端及 Android 验证边界保持不变。
