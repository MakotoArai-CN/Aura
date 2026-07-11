# Listen1 Desktop Tauri

Listen1 的 Tauri 2 重构版。目标是保留 Listen1 原版现代播放器体验，同时补齐本地音乐、桌面歌词、缓存、下载和跨平台构建能力。

## 功能概览

| 模块 | 说明 |
| --- | --- |
| 音乐平台 | 网易云、QQ 音乐、酷狗、酷我、哔哩哔哩、咪咕、千千 |
| 本地音乐 | 支持 MP3、FLAC、M4A、MP4 音频、AAC、OGG、OPUS、WAV、AIFF、WEBM；读取封面、标题、歌手、专辑和内嵌歌词 |
| 播放器 | 现代底部播放器、展开播放页、播放队列、歌词滚动、桌面浮动歌词 |
| 下载 | 歌曲列表提供下载按钮，默认保存到系统音乐目录，可在设置页修改 |
| 智能缓存 | 默认缓存到用户目录 `Listen1/Cache`，上限 2GB；按播放次数、最近访问时间和文件大小评分，长期低分缓存自动清理 |
| 系统能力 | 托盘、窗口控制、全局快捷键、开机自启动 |
| 主题 | 现代黑、现代白、Liquid Glass；主题切换带点击处圆形扩散动画 |
| 构建 | 桌面三平台自动工作流，Android/iOS 手动工作流入口 |

## 技术栈

| 层级 | 方案 |
| --- | --- |
| 桌面框架 | Tauri 2 + Rust |
| 前端 | Svelte 5 + Vite 6 |
| 包管理器 | Bun |
| 音频播放 | Howler.js |
| 后端请求 | Rust `reqwest` 统一代理并注入平台请求头 |
| 本地音乐标签 | Rust `lofty` |
| 本地能力 | Tauri dialog/fs/shell/store/autostart/global-shortcut 插件 |

## 开发

```bash
cd listen1_tauri
bun install
bun run tauri:dev
```

`tauri:dev` 会先构建前端，再使用 Tauri custom protocol 启动，避免在纯 Vite WebView 下缺失 IPC。

## 构建

桌面构建：

```bash
bun run tauri:build
```

移动端命令：

```bash
bun run tauri:android:init
bun run tauri:android:build

bun run tauri:ios:init
bun run tauri:ios:build
```

GitHub Actions 中：

- `desktop` 自动构建 Windows、macOS、Linux 包。
- `android` 和 `ios` 通过 `workflow_dispatch` 手动触发，适合配置 SDK/签名后使用。

## 缓存策略

音频缓存位于 Rust 流代理层：

1. 播放远程歌曲时先检查缓存。
2. 命中缓存时从本地文件返回，并支持 HTTP Range。
3. 未命中时请求远端，完整响应成功后写入缓存。
4. 每次写入后统计总大小，超过上限时按评分从低到高清理。

评分由播放命中次数、最近访问时间和文件大小计算，并标记为 `hot`、`warm`、`cold` 三类。默认上限为 2GB，可在设置页调整。

## 本地音乐

本地文件通过系统文件选择器导入，持久化到本地歌单。播放时不会直接把 `file://` 暴露给 WebView，而是走本地流服务：

```text
file:///C:/Music/demo.flac
-> http://127.0.0.1:<port>/stream/<encoded-url>
```

这样可以规避 WebView 的 `ERR_UNKNOWN_URL_SCHEME`，同时支持 FLAC、MP4 音频等格式播放。

## 项目结构

```text
listen1_tauri/
  src/                Svelte 前端
  src-tauri/src/      Rust 后端命令、代理、窗口、缓存、本地音乐标签
  src-tauri/capabilities/
                      Tauri 权限声明
  public/             静态资源
  dist/               前端构建产物
```

## 验证建议

```bash
bun run build
cd src-tauri
cargo check --features custom-protocol
```

涉及 WebView、IPC、窗口按钮、本地音乐播放、桌面歌词和下载时，应使用真实 Tauri WebView 验证，不只用浏览器预览。
