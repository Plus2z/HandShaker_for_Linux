# HandShaker for Linux 项目记忆与技术调研规范 (Project Memory & Architecture Spec)

> 本文档基于 2026 年针对 HandShaker 多端维护现状及 Linux 移植可行性调研会话（Gemini 共享会话）提炼而成，作为后续设计、协议逆向与开发的核心记忆依据。

---

## 一、 项目背景与现状梳理

1. **官方历史**：
   - 锤子科技（Smartisan）曾官方推出 macOS 端（SmartFinder，后更名 HandShaker）与 Windows 端（基于 .NET / WPF 架构），未曾推出过 Linux 桌面端。
2. **社区维护现状（UP主“禄眠” / rianlu）**：
   - GitHub 上存在三端维护项目：
     - `rianlu/handshaker-android-maintained`（Android 移动端）
     - `rianlu/handshaker-mac-maintained`（macOS 桌面端）
     - `rianlu/handshaker-windows-maintained`（Windows 桌面端）
   - **核心特征**：社区版基于“逆向修补与打补丁”（Smali 字节码、.NET IL 汇编、Mac 二进制 patch）而非源码重写。
   - 目前开源社区**不存在任何 Linux 原生或非官方维护版本**。

---

## 二、 核心技术路线决议：协议级 Clean-room 重实现

| 路线方案 | 具体做法 | 可行性与代价 | 结论 |
| :--- | :--- | :--- | :--- |
| **路线 A：协议级重实现（推荐）** | **完全兼容 Android 维护版客户端**，在 Linux 端从零用现代技术栈重写通信协议与桌面 UI | **高可行性**。手机端无需修改，协议逻辑可从 Smali/IL/诊断代码提取，无原厂代码版权纠纷 | **唯一可行且最佳方案** |
| **路线 B：Windows 版 Wine/Mono 兼容** | 尝试用 Mono 或 Wine 运行 Windows 端的 IL / .NET 程序 | **极易失败**。原版重度依赖 WPF 与 Win32 USB 私有驱动，Mono 不支持 WPF，Wine 下 USB AOA 穿透极不稳定 | **放弃** |
| **路线 C：macOS 版 Darling 运行** | 尝试在 Linux 上通过 Darling 运行 macOS Objective-C 二进制 | **不可行**。重度依赖 Cocoa 与 IOKit USB 驱动栈，Linux 兼容层无法支持 | **放弃** |

---

## 三、 系统分层架构设计

```
┌────────────────────────────────────────────────────────┐
│                   桌面 UI 表现层                       │
│    (Tauri + Web CSS 拟物还原  /  Avalonia UI XAML)    │
│  - 欢迎/扫码翻转页  - 分类导航栏  - 底部存储条        │
│  - 相册时间线+QuickLook  - 便签式剪贴板  - 自定义标题栏 │
└───────────────────────────┬────────────────────────────┘
                            │ (IPC / API)
┌───────────────────────────┴────────────────────────────┐
│                  核心协议与业务引擎                    │
│                 (Rust / Go / Python)                   │
│  - 握手管理 (QR Code, Token, Capabilities)             │
│  - 帧封装/拆包 (Framing, Magic, Length, Opcode)        │
│  - 业务模块 (文件/目录树, 缩略图管道, 剪贴板同步, FUSE)   │
└───────────────────────────┬────────────────────────────┘
                            │ (双通道抽象)
             ┌──────────────┴──────────────┐
             ▼                             ▼
┌──────────────────────────┐  ┌──────────────────────────┐
│     Wi-Fi 局域网通道     │  │      USB AOA 直连通道     │
│  - TCP Socket 数据流     │  │  - libusb / rusb 驱动     │
│  - UDP 局域网广播发现    │  │  - AOA 握手切换 (Req 51-53)│
│                          │  │  - Bulk IN/OUT 批量端点   │
└──────────────────────────┘  └──────────────────────────┘
```

---

## 四、 UI 视觉资产与复刻策略（“原汁原味”锤味体验）

1. **资产提取途径**：
   - **macOS 端资源（`Contents/Resources/` & `Assets.car`）**：提取全套 2x Retina 高清拟物切图（手机外壳渲染图、拟物文件夹图标、金属工具栏、指示灯、插拔音效）。
   - **Windows 端资源（反编译 BAML -> XAML）**：使用 ILSpy/dnSpy 还原 100% 精确的布局参数（各栏边距、精确十六进制色号、圆角半径、Hover/Pressed 动画逻辑）。
2. **UI 框架与技术栈决议（已确认）**：
   - **已选定路线：Tauri v2 (Rust 核心引擎 + Web 前端 CSS3)**。
   - **选型理由**：
     - **视觉表现力 100%**：锤子经典拟物风格（内阴影 `box-shadow: inset`、多层渐变边框、3D 翻转卡片动画）在 CSS3 中最容易像素级还原。
     - **后端高性能与稳健性**：Rust（`tokio` + `rusb`）原生处理 TCP 局域网并发流、缩略图异步管道及 Linux USB AOA（`libusb`）控制与批量传输，内存占用极低。
     - **打包分发友好**：原生支持 Linux 下的 AppImage、deb、rpm 与 Flatpak 构建。
   - *备选对比（已放弃）：Avalonia UI (.NET 8 / C#) 与 Qt6 / QML。*
3. **锤味灵魂的 5 大核心 UI 元素**：
   - ① 欢迎与连接引导页（手机拟物线框 + 扫码翻转卡片）。
   - ② 左侧导航栏与底部彩色存储条（应用/图片/视频/音乐/其他分段比例与配色）。
   - ③ 相册时间线与拍立得白边阴影网格，支持空格键快速预览（Quick Look）。
   - ④ 剪贴板历史便签（便签纸质感、手机与电脑双向同步）。
   - ⑤ 一体化浅灰渐变无边框自定义标题栏。

---

## 五、 五步实施计划路线图

- **阶段一：协议逆向与报文梳理（1-2周）**
  - 克隆 `rianlu` 的三端仓库，分析 Smali 与 IL 代码，提取握手格式、消息帧头、指令集（Opcode）。
  - 参考 Mac 端 `diagnostics/java/` 及日志追踪逻辑，形成完整的协议规范文档。
- **阶段二：Wi-Fi 局域网连接 CLI 原型（1-2周）**
  - 编写最小 CLI 工具，实现终端二维码生成与手机扫码连接。
  - 完成 TCP 握手与能力位协商（上报能力解锁剪贴板页签）。
  - 实现存储容量查询、列出目录、拉取/推送文件核心指令。
- **阶段三：桌面 UI 与高频功能闭环（2-4周）**
  - 搭建桌面端框架，还原经典拟物界面。
  - 实现相册流与多线程缩略图异步缓存机制。
  - 接入 Linux 剪贴板（兼顾 Wayland 与 X11）。
- **阶段四：攻克 Linux USB AOA 直连模式（2-3周）**
  - 编写 udev 规则赋予非 root 权限，规避 `gvfs-mtp` / `kio-mtp` 设备接口占用冲突。
  - 调用 libusb 执行 AOA 控制传输握手（51/52/53），切入配件模式（0x18D1:0x2D00/2D01）。
  - 将应用层协议流从 TCP 切换至 USB Bulk IN/OUT 端点。
- **阶段五：Linux 深度集成与分发（持续）**
  - FUSE 虚拟文件系统挂载（直接在系统文件管理器中浏览操作手机文件）。
  - AppImage / Flatpak 打包与构建流水线。

---

## 六、 关键坑点与防护对策

1. **MTP 自动抢占 USB 接口**：
   - 现象：GNOME/KDE 插入手机时自动占用 USB 接口，导致 `LIBUSB_ERROR_BUSY`。
   - 对策：调用 `libusb_set_auto_detach_kernel_driver`，并配套 udev 规则忽略特定 AOA 设备。
2. **Wayland 剪贴板安全限制**：
   - 现象：Wayland 禁止后台程序随意监听全局剪贴板。
   - 对策：采用窗口获得焦点（Focus In）时拉取同步，或快捷键触发同步机制。
3. **资源版权合规**：
   - 声明个人学习与兼容性研究用途，或提供一键从官方/维护版离线提取切图的安装脚本。
