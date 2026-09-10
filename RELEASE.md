纯 Rust 单文件，GUI / CLI 双模式，解压即用。

## 下载

| 文件 | 平台 | 模式 |
|---|---|---|
| `NekoSportsWorldTool-win-x64.zip` | Windows 10/11 | GUI + CLI |
| `NekoSportsWorldTool-macos-arm64.tar.gz` | macOS (Apple Silicon) | GUI + CLI |
| `NekoSportsWorldTool-linux-x64.tar.gz` | Linux x64 | CLI |
| `NekoSportsWorldTool-linux-aarch64.tar.gz` | Linux ARM64 | CLI |

## 快速上手

```bash
# GUI：双击 exe

# CLI：
./NekoSportsWorldTool login --user <手机号> --pass <密码> --remember
./NekoSportsWorldTool run        # 一键跑步
./NekoSportsWorldTool help       # 全部命令
```

## 青龙定时任务

```
cron: 0 30 8 * * *
命令: /path/to/NekoSportsWorldTool run
```

首次手工 `login --remember` 一次，之后会话失效自动重登。
