# CyberAPI 核心体验演进技术方案设计文档：桌面端更新交互、独立关于对话框、响应体默认美化及 URL QueryString 直接编辑

- **文档版本**: 1.0.0
- **创建日期**: 2026-11-12
- **状态**: 方案设计已冻结 (Architecture Design Complete)
- **作者**: Teamwork Worker 1 (Architecture & System Design)
- **目标平台**: CyberAPI GPUI 跨平台客户端 (macOS / Windows / Linux 桌面端及 Wasm 浏览器端)
- **关联需求**: R1 (技术方案设计), R2 (桌面端标题栏更新体验), R3 (独立关于对话框), R4 (响应体默认美化), R5 (URL QueryString 直接编辑与兼容存储)
- **前置探索输入**: Explorer 1 (`teamwork_explorer_m1_1`), Explorer 2 (`teamwork_explorer_m1_2`), Explorer 3 (`teamwork_explorer_m1_3`)

---

## 目录

1. [背景与全局设计原则 (Background & Global Design Principles)](#1-背景与全局设计原则-background--global-design-principles)
   - 1.1 [演进背景与业务价值](#11-演进背景与业务价值)
   - 1.2 [全局架构契约与分层设计](#12-全局架构契约与分层设计)
   - 1.3 [GPUI 设计与交互规范落地方案](#13-gpui-设计与交互规范落地方案)
   - 1.4 [跨平台支持边界与条件编译规范](#14-跨平台支持边界与条件编译规范)
   - 1.5 [代码零侵入原则与设计约束](#15-代码零侵入原则与设计约束)
2. [需求 R2：桌面端标题栏更新体验方案 (Desktop Titlebar Update Experience)](#2-需求-r2桌面端标题栏更新体验方案-desktop-titlebar-update-experience)
   - 2.1 [标题栏组件架构与跨平台布局拓扑](#21-标题栏组件架构与跨平台布局拓扑)
   - 2.2 [更新状态机与底层生命周期分析](#22-更新状态机与底层生命周期分析)
   - 2.3 [标题栏更新入口与检测中（Loading）指示器](#23-标题栏更新入口与检测中loading指示器)
   - 2.4 [Popover 更新浮窗交互与结构设计](#24-popover-更新浮窗交互与结构设计)
   - 2.5 [原位进度指示（In-place Progress）与下载生命周期](#25-原位进度指示in-place-progress与下载生命周期)
   - 2.6 [跨平台安装与重启生命周期实现](#26-跨平台安装与重启生命周期实现)
3. [需求 R3：独立“关于（About）”对话框方案 (Standalone About Dialog)](#3-需求-r3独立关于about对话框方案-standalone-about-dialog)
   - 3.1 [元信息架构设计与读取机制](#31-元信息架构设计与读取机制)
   - 3.2 [唤起入口与全局导航路由](#32-唤起入口与全局导航路由)
   - 3.3 [UI 视觉结构与组件层级规范](#33-ui-视觉结构与组件层级规范)
   - 3.4 [模态对话框构建模式与平台兼容性](#34-模态对话框构建模式与平台兼容性)
4. [需求 R4：响应体格式化默认美化方案 (Response Body Default Pretty Formatting)](#4-需求-r4响应体格式化默认美化方案-response-body-default-pretty-formatting)
   - 4.1 [历史背景与性能瓶颈深度复盘](#41-历史背景与性能瓶颈深度复盘)
   - 4.2 [默认模式变更机制与数据流分析](#42-默认模式变更机制与数据流分析)
   - 4.3 [与 JSONPath 过滤功能的深度协同](#43-与-jsonpath-过滤功能的深度协同)
   - 4.4 [用户手动切换与记忆机制设计](#44-用户手动切换与记忆机制设计)
   - 4.5 [极限边界、容错与多级降级安全网设计](#45-极限边界容错与多级降级安全网设计-exhaustive-safeguards--fallbacks)
5. [需求 R5：URL QueryString 模式可选与兼容存储方案 (Configurable URL QueryString Mode & Storage Compatibility)](#5-需求-r5url-querystring-模式可选与兼容存储方案-configurable-url-querystring-mode--storage-compatibility)
   - 5.1 [需求定位：设置项驱动的双模式共存](#51-需求定位设置项驱动的双模式共存)
   - 5.2 [偏好设置模型与设置界面设计 (`TextPrefs.query_in_url`)](#52-偏好设置模型与设置界面设计-textprefsquery_in_url)
   - 5.3 [两种模式下的编辑器 UI 与 Tab 栏行为](#53-两种模式下的编辑器-ui-与-tab-栏行为)
   - 5.4 [模板感知型 URL 解析与组装机制 (`split_uri_query` / `split_query_pairs`)](#54-模板感知型-url-解析与组装机制-split_uri_query--split_query_pairs)
   - 5.5 [保存与加载双向兼容设计（单一底层存储结构）](#55-保存与加载双向兼容设计单一底层存储结构)
   - 5.6 [模式热切换、参数元数据保留与下游零侵入兼容](#56-模式热切换参数元数据保留与下游零侵入兼容)
6. [实施路径与任务分解清单 (Implementation Roadmap & Checklist)](#6-实施路径与任务分解清单-implementation-roadmap--checklist)
   - 6.1 [修改文件全景清单](#61-修改文件全景清单)
   - 6.2 [详细测试用例规划](#62-详细测试用例规划)
   - 6.3 [国际化与本地化文案字典变更](#63-国际化与本地化文案字典变更)
   - 6.4 [面向用户文档同步规划](#64-面向用户文档同步规划)
   - 6.5 [发布与回归验证矩阵](#65-发布与回归验证矩阵)

---

## 1. 背景与全局设计原则 (Background & Global Design Principles)

### 1.1 演进背景与业务价值

CyberAPI 是基于 Rust 与 GPUI 框架重构的新一代高性能、跨平台 API 开发与调试工作台（对标 Postman / Insomnia），支持以同一套核心代码同时驱动高性能桌面端（macOS、Windows、Linux）与全功能 WebAssembly 浏览器端。

在 0.4.x 系列版本发布后，随着核心请求执行、离线优先多端同步（Turso + SQLite / PostgreSQL / MySQL）以及大载荷渲染性能的持续优化，开发团队收集到了关于日常交互体验与研发效率的集中反馈：
1. **客户端更新割裂 (R2)**：桌面端现有版本检查以突兀的整屏模态弹窗或隐藏在设置菜单深处为主，缺乏无缝的沉浸式感知。用户在工作时需要即时了解更新状态、轻量阅读更新日志，并在下载期间获得原位视觉反馈。
2. **元信息与诊断缺失 (R3)**：缺乏独立的“关于”窗口，无法一览 Commit 哈希、构建时间及环境架构，阻碍了用户排查环境问题与提交 Bug 时的元信息复制。
3. **响应体查看体验逆向 (R4)**：历史上因规避巨型压缩 JSON 的软折行性能抖动而默认回退为 Raw 模式，导致 95% 以上的标准 API 调试场景中，用户每次发起请求都必须手动点击“美化”，且阻碍了 JSONPath 过滤栏的开箱可用性。
4. **URL 参数编辑模式个性化诉求 (R5)**：不同开发者对 Query 参数的编辑习惯不同——部分用户习惯默认的独立 Key-Value 表格编辑模式（方便逐行勾选启停、撰写参数说明），另一部分用户则希望直接在 URL 输入框中编写和保留完整的 `?key=val` QueryString（不使用独立的 Query 表格编辑器）。因此需要在设置中提供模式选择（**默认保持当前表格编辑模式**），并在保存与读取时保持底层数据结构完全兼容，确保单人随时切换模式或团队成员混用不同模式时无缝互通。

本方案旨在针对上述四项核心需求，完成端到端的系统架构分析、状态机演进模型、UI 交互拓扑与零迁移存储向后兼容设计，为后续生产级编码提供严密的落地蓝图。

### 1.2 全局架构契约与分层设计

CyberAPI 的系统架构严格遵循四层正交解耦模型：

```
┌────────────────────────────────────────────────────────────────────────┐
│                   Layer 4: UI & Shell Presentation                      │
│   cyberapi-gui (Desktop Application)  │   cyberapi-web (Wasm SPA)      │
│   src/root.rs, src/views/*, src/states/* (GPUI Entities & Components)  │
├────────────────────────────────────────────────────────────────────────┤
│                   Layer 3: Application & Coordination                  │
│   cyberapi-service (Execution dispatch, Code generation, MCP runtime)  │
├────────────────────────────────────────────────────────────────────────┤
│                   Layer 2: Storage & Sync Abstractions                 │
│   cyberapi-store-turso (Desktop local LibSQL/Turso)                    │
│   cyberapi-store-sql (Server PostgreSQL / MySQL)                       │
│   cyberapi-store-bridge (Web RPC over HTTP / WebSocket)                │
├────────────────────────────────────────────────────────────────────────┤
│                   Layer 1: Domain Core & Data Models                   │
│   cyberapi-core (HttpRequest, Collection, Env, Version, Format, Assert)│
└────────────────────────────────────────────────────────────────────────┘
```

各层级职责与边界契约如下：
- **`cyberapi-core`**：核心纯逻辑层，绝对禁止依赖 UI 库（`gpui`）及平台特定的图形服务。所有涉及 URL 分割、模板语法求值、响应体格式化算法、模型序列化/反序列化（`serde`）均收敛于此。
- **`cyberapi-store-*`**：存储实现层。保证存储数据的无损读写，对外暴露统一的异步 Trait 接口。
- **`cyberapi-service`**：调度引擎层，编排网络请求流转、代码导出及测试断言求值。
- **`cyberapi-gui` / `cyberapi-web`**：展示层，基于 GPUI 实体（`Entity<T>`）与上下文（`Context<T>`, `App`, `Window`）驱动视图响应式渲染。

### 1.3 GPUI 设计与交互规范落地方案

根据 `nsh-design-guides` 与 `gpui-component` 设计规范，CyberAPI 的人机交互遵循以下严苛准则：

1. **语义化主题令牌 (Semantic Tokens Only)**：
   - 严禁在视图代码中直接使用裸色硬编码（如十六进制色值或随机 RGB）。所有文本、边框、背景及悬停态必须取自 `theme.muted`, `theme.secondary`, `theme.border`, `theme.primary`, `theme.accent`。
2. **轻量与防打断原则 (Lightweight Overlays)**：
   - 桌面端操作应尽可能减少阻塞性模态（Blocking Modal Dialog）的滥用。对于版本更新、快捷提示等辅助流，优先采用锚定在触发源附近的 `Popover`，支持点击外部空白（Overlay Closable）及 `Escape` 键快速销毁。
3. **确定性状态动画与无阻塞更新**：
   - 进度变化、加载旋转（Loading Spinners）必须保证 60fps 顺滑，通过不可阻塞的异步通道（`mpsc::unbounded`）解耦计算与绘制，杜绝主 UI 线程重布局卡顿。
4. **键盘第一公民 (Keyboard Accessibility)**：
   - 模态对话框与弹出层必须严格支持全局按键拦截：`Escape` 退出、`Tab` 键轮询焦点、`Enter` 激活主动作（Primary Button）。

### 1.4 跨平台支持边界与条件编译规范

由于 CyberAPI 跨越桌面平台与浏览器沙箱，不同功能模块具有清晰的平台边界：

| 功能特性 | macOS 桌面端 | Windows / Linux 桌面端 | Web 浏览器端 (Wasm) |
|---|---|---|---|
| **标题栏更新指示器 (R2)** | 支持 (无原生控制键冲突) | 支持 (避开客户端窗口控制按钮) | **禁用** (浏览器无本地客户端更新) |
| **安装包原位替换与重启 (R2)** | 支持 (DMG 挂载 + `ditto` + 脚本自重启) | 部分支持 (落地 Downloads 并交由系统安装器) | **禁用** |
| **独立“关于”对话框 (R3)** | 支持 (应用菜单 + 标题栏齿轮 + 设置项) | 支持 (标题栏齿轮 + 设置项) | 支持 (标题栏齿轮 + 设置项，Web 标记) |
| **响应体默认美化 (R4)** | 支持 (本地存储 + 流式格式化) | 支持 (本地存储 + 流式格式化) | 支持 (`localStorage` 持久化) |
| **URL QueryString 直编 (R5)** | 全平台统一支持 | 全平台统一支持 | 全平台统一支持 |

在 Rust 源码中，所有桌面专有逻辑必须使用严格的条件编译门禁：
```rust
#[cfg(not(target_arch = "wasm32"))]
// 桌面端专用更新管理与系统交付逻辑
```

### 1.5 代码零侵入原则与设计约束

作为技术方案设计阶段（M1 Milestone），本任务严格恪守**“代码零侵入原则”**：
- 专注于提供高度详尽、自包含且架构完备的设计文档。
- **不得修改工作区中的任何 Rust 源码文件（包括 `src/` 与 `crates/`）**。
- 保证 Git 工作树绝对整洁，所有技术设计经受多重逻辑自查与交叉验证，可直接作为后续编码实施的标准指导手册。

---

## 2. 需求 R2：桌面端标题栏更新体验方案 (Desktop Titlebar Update Experience)

### 2.1 标题栏组件架构与跨平台布局拓扑

#### 2.1.1 标题栏层级与组件挂载
在 CyberAPI 桌面端中，主窗口顶部的统一标题栏由 `src/root.rs`（第 679–706 行）基于 `gpui-component::TitleBar` 声明构建：

```rust
// src/root.rs 标题栏布局结构
TitleBar::new().child(
    h_flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .child(
            // 左侧/中间区域：项目名称与工作区面包屑
            h_flex()
                .flex_1()
                .w(px(0.))
                .min_w_0()
                .overflow_hidden()
                .child(self.render_title(cx)),
        )
        .child(
            // 右侧动作区域：更新按钮、设置齿轮
            h_flex()
                .flex_none()
                .pr_2()
                .gap_1()
                .items_center()
                .children(update_chip(cx))
                .child(self.render_menu(cx)),
        ),
)
```

#### 2.1.2 跨平台几何差异与安全避让
在底层 `gpui-component-0.9.0/src/title_bar.rs` 中，标题栏定义了标准高度与平台相关的内边距：
- **统一高度**：`TITLE_BAR_HEIGHT = px(34.)`。
- **macOS 平台**：
  - 左侧保留 `TITLE_BAR_LEFT_PADDING = px(80.)`，专门用于避让 macOS 系统的“红绿灯”原生交通灯控件（Close, Minimize, Zoom）。
  - 右侧无任何系统按钮，右侧内边距保持轻量 `px(8.)`。
- **Windows 与 Linux 平台**：
  - 左侧 `TITLE_BAR_LEFT_PADDING = px(12.)`。
  - 右侧挂载了自定义的无边框客户端控制组件 `WindowControls`（包含最小化、最大化/向下还原、关闭三个控制按钮），占宽约 138px。

#### 2.1.3 更新指示器的黄金锚点定位
在当前的 DOM 树中，更新组件挂载在右侧操作流：
`h_flex().children(update_chip(cx)).child(self.render_menu(cx))`
- 该位置处于**设置齿轮按钮（`render_menu`）的紧邻左侧**。
- 在 macOS 上，它位于窗口最右上角操作区的黄金视觉带；在 Windows/Linux 上，它位于系统三键（最小化/最大化/关闭）的左侧并与齿轮按钮并列。
- **此位置在全平台均拥有最高的视觉发现率，且绝不与系统窗体控件产生重叠冲突**。

### 2.2 更新状态机与底层生命周期分析

CyberAPI 现有的更新子系统具备极强的分层解耦能力：

```
┌────────────────────────────────────────────────────────────────────────┐
│                        cyberapi_core::update                           │
│  Manifest (latest.json), Asset (url, sha256, size, kind), is_newer     │
└──────────────────────────────────▲─────────────────────────────────────┘
                                   │
┌──────────────────────────────────┴─────────────────────────────────────┐
│                         src/states/update.rs                           │
│  UpdateState (GPUI Entity)                                             │
│  Phase: Idle ──► Checking { manual } ──► Downloading ──► Installed    │
│  Actions: check(manual), skip(), download_and_install(), dismiss()     │
└──────────────────────────────────▲─────────────────────────────────────┘
                                   │
┌──────────────────────────────────┴─────────────────────────────────────┐
│                        src/update_install.rs                           │
│  macOS: in_place (DMG mount, ditto swap, relaunch script)              │
│  Windows/Linux: hand_over (Save to Downloads, launch OS installer)     │
└────────────────────────────────────────────────────────────────────────┘
```

#### 2.2.1 状态枚举与核心转移契约
`UpdatePhase`（定义于 `src/states/update.rs`）：
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdatePhase {
    Idle,
    Checking { manual: bool },
    Downloading,
    Installed,
}
```

#### 2.2.2 自动检测周期与跳过版本机制
- **静默检测**：应用启动 5 秒（`START_DELAY`）后触发 `check_at_start(cx)`。如果用户启用了自动更新且距离上次检查已超过 48 小时（`CHECK_INTERVAL`），则在后台静默发起 HTTP 请求获取 `MANIFEST_URL`（即 `latest.json`）。
- **版本比对**：通过 `cyberapi_core::update::is_newer` 比较语义化版本号（支持 Nightly 构建的时间戳及提交比对，如 `0.2.2-nightly.20261024.137a858`）。
- **跳过过滤**：若用户曾对该版本执行过“跳过更新”，该版本的 Version 字符串将记录在 `AppPrefs.updates.skipped` 中。自动检测命中跳过版本时，直接重置为 `Idle`，不打扰用户；若用户在菜单中点击“检查更新”（`manual = true`），则无视跳过标记强制展示。

### 2.3 标题栏更新入口与检测中（Loading）指示器

#### 2.3.1 状态驱动的标题栏渲染模型
现有的 `update_chip(cx)` 仅在检测到新版本可用时渲染高亮 Chip，在 `Checking` 或 `Idle` 状态下均返回 `None`。为满足需求 R2，应将其重构为常驻/响应式的标题栏更新视图组件（Titlebar Update View）：

```
[Phase: Checking] ──► 渲染微型 Loading 状态（旋转轮圈或带 loading 态的微型胶囊）
       │
       ├─► [无新版本 / 检查失败] ──► 渐隐回退至 Idle，若为手动检查则 Toast 提示
       │
       └─► [检测到新版本] ──► 渲染带有版本徽标的交互式触发按钮（v0.3.0 ✨）
                                      │
                                      ▼ (用户点击)
                               [展开 Popover 浮窗]
```

#### 2.3.2 UI 表现与交互设计细节
1. **Checking 状态（检测中）**：
   - 呈现为一个微型圆角胶囊组件：`h_flex().items_center().gap_1().px_2().py_0p5().rounded_md()`。
   - 挂载 `ProgressCircle::new("update-checking").xsmall().indeterminate(true)`，或者使用带有微旋转动画的 `Icons::Loader2` 图标。
   - 文本辅助：在鼠标悬停时展示 Tooltip：“正在检查更新...”。
2. **Available 状态（新版本就绪）**：
   - 呈现为醒目的强调胶囊：`bg(theme.accent).text_color(theme.accent_foreground)`。
   - 包含闪烁的小星点或向上更新箭头图标 `Icons::ArrowUpCircle` 与版本号文本（如 `v0.3.0`）。
   - 拥有平滑的 Hover 放大微动效（Scale 1.02，背景色微亮）。

### 2.4 Popover 更新浮窗交互与结构设计

#### 2.4.1 Popover 相比 Modal 对话框的核心优势
需求 R2 明确要求：
> “检测到新版本后点击按钮弹出 popover 显示更新说明，提供‘跳过更新’与‘更新并重启’两个选项；点击‘更新并重启’后关闭 popover，在 titlebar 内原位显示下载与更新进度。”

采用 `gpui-component::popover::Popover` 的关键优势在于：
- **零破坏性**：用户无需被一个居中的大模态挡住整个工作区，不打断当前的 API 调试思维。
- **空间连续性**：浮窗直接吸附于标题栏更新胶囊下方，用户的视线与操作焦点保持高度收敛。

#### 2.4.2 Popover 挂载与边界防溢出机制
```rust
Popover::new("title-update-popover")
    .anchor(Anchor::TopRight)
    .offset(px(6.))
    .arrow(true)
    .overlay_closable(true)
    .trigger(update_trigger_button)
    .content(move |popover_state, window, cx| {
        render_update_popover_card(popover_state, window, cx)
    })
```
- **尺寸控制**：固定宽度 `px(400.)`，最大高度 `px(380.)`。
- **防溢出保护**：当主窗口宽度较窄（如仅 800px）时，应用 `snap_to_window_with_margin(px(10.))` 机制，确保 Popover 边缘始终距离窗口右边界至少 10px，不被视口截断。

#### 2.4.2.1 标题栏窗口拖拽与鼠标点击竞态阻断 (Window Dragging vs Click Fix)
在 GPUI 原生 `TitleBar`（`gpui-component/src/title_bar.rs`）的事件处理模型中：
- 默认情况下，标题栏背景接收到鼠标左键按下（`MouseButton::Left`）时会将内部标志位 `should_move` 置为 `true`；随后的鼠标微小移动会直接触发 `window.start_window_move()`。
- `start_window_move()` 会捕获底层视窗指针（Capture Pointer），导致标题栏内挂载的更新 Chip 或 Popover 触发按钮无法接收到完整的 `Click` 事件（鼠标按下被拖拽接管，释放事件被吞没），出现“点击更新按钮经常失效或变成拖拽窗口”的恶性体验缺陷。
- **确切阻断方案**：
  更新指示器外层容器必须显式声明 `.occlude()` 遮挡层，并在 `on_mouse_down` 事件中调用 `cx.stop_propagation()` 阻止事件向上冒泡至 `TitleBar`：
  ```rust
  div()
      .id("titlebar-update-container")
      .occlude() // 阻止穿透，告知 GPUI 该区域不属于窗口背景
      .on_mouse_down(MouseButton::Left, |_, cx| {
          cx.stop_propagation(); // 阻断冒泡，杜绝 TitleBar 触发 should_move 与 start_window_move
      })
      .child(
          // 挂载 Popover 触发器或原位进度控制器
          render_update_chip_element(state, window, cx)
      )
  ```
  通过此层硬性阻断，彻底杜绝窗口拖拽与组件点击的事件竞争，确保更新 Chip 的 Hover、Click 及 Popover 展开 100% 确定性触发。

#### 2.4.3 Popover 内部视觉布局卡片
卡片分为三个标准纵向分区：

```
┌────────────────────────────────────────────────────────┐
│ [🚀 新版本就绪: v0.3.0]                   (2026-11-12) │
│ 当前版本: 0.2.1 (abc1234)  大小: 48.2 MB               │
├────────────────────────────────────────────────────────┤
│ 更新日志 (Markdown 渲染, 可滚动的视口 max_h: 220px)    │
│ - 新增: URL QueryString 直接编辑                       │
│ - 新增: 独立关于对话框                                 │
│ - 优化: 响应体默认启用 Pretty 美化排版                 │
│ - 修复: 某些特殊响应体在高并发下的内存抖动             │
├────────────────────────────────────────────────────────┤
│ [ 跳过此版本 ]                         [ 更新并重启 ]   │
│ (Button::ghost)                      (Button::primary) │
└────────────────────────────────────────────────────────┘
```

- **“跳过此版本”交互与精确语义 (Skip Update Semantics)**：
  - 点击后，调用 `state.skip(cx)`，将目标版本的精确版本号字符串 `info.version` 写入持久化存储中的 `AppPrefs.updates.skipped`。
  - **版本精准匹配语义**：更新状态机在自动静默检查时通过以下代码判定跳过：
    ```rust
    let skipped = app.read(cx).prefs().updates.skipped == info.version;
    if manual || !skipped {
        this.available = Some(info);
        cx.emit(UpdateEvent::Found { manual });
    }
    ```
    - 所谓“跳过”，仅严格阻断**完全相同版本号字符串**（Exact String Match，例如 `"0.3.0"`）的后续自动静默提醒。
    - **不影响后续任何新版本**：一旦官方后续发布了哪怕一个微小补丁版（如 `"0.3.1"`）或小版本（如 `"0.4.0"`），由于 `info.version != skipped`，系统在后台自动检测到后将**立刻恢复提醒**。
    - **不影响手动检查**：用户在设置菜单或关于窗口中手动点击“检查更新”（`manual = true`）时，系统无条件绕过 `skipped` 过滤并弹出更新详情。
  - 点击跳过后，立即关闭 Popover，标题栏更新指示器恢复为隐藏（`Idle`）。
- **“更新并重启”交互**：
  - 点击后，立即调用 `popover_state.close(window, cx)` 关闭浮窗，将交互归还给主窗口。
  - 异步分发 `state.download_and_install(cx)`，状态机无缝跃迁至 `UpdatePhase::Downloading`。

### 2.5 原位进度指示（In-place Progress）与下载生命周期

#### 2.5.1 标题栏原位形态变换
当 `UpdatePhase::Downloading` 激活时，原先挂载 Popover 的触发按钮在标题栏中**就地变换**为紧凑型下载进度控制器：

```rust
// 伪代码：标题栏原位进度呈现
h_flex()
    .items_center()
    .gap_1p5()
    .px_2()
    .py_1()
    .rounded_md()
    .bg(theme.secondary)
    .border_1()
    .border_color(theme.border)
    .child(
        ProgressCircle::new("titlebar-dl-circle")
            .xsmall()
            .value((downloaded as f32 / total.max(1) as f32) * 100.0)
    )
    .child(
        div()
            .text_xs()
            .font_semibold()
            .text_color(theme.foreground)
            .child(format!("{}%", pct))
    )
    .tooltip(move |window, cx| {
        Tooltip::new(format!(
            "{}: {} / {}",
            t!("update.downloading"),
            format_bytes(downloaded),
            format_bytes(total)
        ))
    })
```

#### 2.5.2 60fps 高性能响应式流分发
- 下载底层经由 `cyberapi_http` 流式拉取字节流。
- `UpdateState` 维持一个无界通道 `tokio::sync::mpsc::unbounded_channel::<(u64, u64)>()`。
- 下载任务每次接收到网络数据包（Chunk）时向通道发送已读字节与总字节；GPUI 任务以节流模式（每 16ms 轮询一次通道最新进度）调用 `cx.notify()` 触发重绘。
- **计算与渲染彻底解耦**，即使以 100MB/s 极速下载，也不会造成 UI 线程丢帧或卡顿。

#### 2.5.3 原位下载失败处理与重试状态机 (In-Place Download Failure Handling)
原先的设计中，若原位下载因网络超时、断流或 SHA-256 哈希校验失败而中断（分发 `UpdateEvent::InstallFailed(error)`），状态机若直接回退至 `Idle`，会导致标题栏进度条突然静默消失，给用户造成“应用崩溃或更新已结束”的严重错觉。
为此，设计完善的失败容错与重试机制：

1. **状态机扩展 (`UpdatePhase::Failed`)**：
   在 `src/states/update.rs` 中，当下载任务失败时，状态机保持 `available` 数据，并进入显式失败态：
   ```rust
   UpdateEvent::InstallFailed(error) => {
       this.phase = UpdatePhase::Failed { error: error.clone() };
       cx.emit(UpdateEvent::InstallFailed(error));
   }
   ```
2. **标题栏错误指示器就地切换**：
   标题栏原位进度条不消失，而是变换为警示 Chip：
   - **视觉呈现**：背景 `theme.danger.opacity(0.15)`，边框 `theme.danger`，文字与图标使用 `theme.danger`。
   - **图标与文案**：`Icons::AlertTriangle` 配合 `t!("update.download_failed")`（“更新下载失败”）。
   - **悬停 Tooltip**：展示具体错误原因（如 `error = "Connection reset by peer"` 或 `"SHA-256 checksum mismatch"`）。
   - **点击交互**：
     - 单击唤起轻量重试浮窗（Popover），提供：
       - **“重试下载”按钮**：重新调用 `state.download_and_install(cx)`，状态回到 `Downloading` 重新拉取。
       - **“在浏览器打开发布页”按钮**：调用 `cx.open_url(&info.page)` 允许用户走官网或 GitHub Release 手动下载。
       - **“忽略”按钮**：将状态重置回 `Idle`，隐藏 Chip。
3. **全局通知系统（`follow_update`）联动**：
   在 `src/views/update_dialog.rs` 的 `follow_update` 中增加对 `InstallFailed` 的通知分发：
   ```rust
   UpdateEvent::InstallFailed(error) => {
       Some(Notification::error(
           t!("update.install_failed_toast", error = error).to_string(),
       ).action(|_, _, _| {
           Button::new("update-retry")
               .small()
               .danger()
               .label(t!("update.retry").to_string())
               .on_click(|_, _, cx| {
                   if let Some(state) = AppStore::global(cx).updates() {
                       state.update(cx, |s, cx| s.download_and_install(cx));
                   }
               })
       }))
   }
   ```
   双重保障确保用户即时获知下载失败的原因并能一键重试。

### 2.6 跨平台安装与重启生命周期实现

在 `download_and_install` 下载完成并通过 SHA-256 哈希校验后，调用 `src/update_install.rs` 的跨平台安装引擎：

#### 2.6.1 macOS 平台：原位 DMG 挂载替换与脚本重拉起
macOS 平台支持真正的“零手动干预、原位热替换”：
1. **DMG 挂载与校验**：
   - 使用系统命令行 `hdiutil attach -nobrowse -readonly <dmg_path>` 挂载至临时挂载点。
   - 严格比对新 bundle 的 `CFBundleIdentifier` 必须等于 `com.bigtree.cyberapi`，杜绝恶意安装包注入。
2. **Ditto 覆盖写入**：
   - 调用 `/usr/bin/ditto <mount>/CyberAPI.app /Applications/CyberAPI.app` 将新应用覆盖到系统应用目录。
   - 调用 `hdiutil detach` 卸载镜像。
3. **安全自启脚本（Relaunch）**：
   - 生成并以独立会话派发后台 Shell 脚本：
     ```bash
     while /bin/kill -0 {current_pid} 2>/dev/null; do
         /bin/sleep 0.1
     done
     /usr/bin/open "/Applications/CyberAPI.app"
     ```
   - CyberAPI 客户端调用 `cx.quit()` 自行优雅退出，后台脚本在感知到原进程死亡后，立即拉起新版 `.app`。

#### 2.6.2 Windows 与 Linux 平台：真实打包格式与系统交接 (System Handover)
根据 CyberAPI 生产构建体系（`AGENTS.md`、`crates/cyberapi-core/src/update.rs` 与 `src/update_install.rs`）：
- **Windows 打包真相**：CyberAPI 在 Windows 平台上发布的是**免安装便携版 Zip 压缩包**（`kinds = &["zip"]`），**并不生成 `.exe` 安装程序或 MSI 安装包**！
- **Linux 打包真相**：发布 `.deb` 二进制安装包及通用 `.tar.gz` 绿色压缩包。

系统交付逻辑与源码 `src/update_install.rs:59-79` 严格对齐：
1. 安装包下载至用户的系统下载目录（`~/Downloads/`）。
2. **Windows 平台交付**：
   - 调用系统资源管理器执行高亮定位：
     ```rust
     Command::new("explorer")
         .arg(format!("/select,{}", path.display()))
         .spawn()
     ```
   - 打开 Windows 资源管理器并自动选中下载好的 `.zip` 文件。
   - 标题栏与通知栏弹出指引消息：“新版本便携包已下载完成，请解压并替换当前 CyberAPI 目录后重新启动应用。”
3. **Linux 平台交付**：
   - 若下载为 `.deb` 包，通过 `xdg-open <path_to_deb>` 唤起系统的软件中心（如 Ubuntu Software）进行安装。
   - 若下载为 `.tar.gz` 压缩包，调用 `xdg-open` 打开其所在目录供用户解压。

---

## 3. 需求 R3：独立“关于（About）”对话框方案 (Standalone About Dialog)

### 3.1 元信息架构设计与读取机制

#### 3.1.1 核心元信息现状与缺失项补充
在现有的 `crates/cyberapi-core/src/version.rs` 中，已经维护了基础的语义化版本及短提交信息：
- `VERSION: &str`：由构建环境中的 `CYBERAPI_VERSION` 或 `CARGO_PKG_VERSION` 提供。
- `COMMIT: Option<&str>`：由 `CYBERAPI_COMMIT` 环境变量提供。
- `describe() -> String`：格式化产出版本摘要（如 `0.2.1 (abc1234)`）。
- `MODEL: u32 = 24`：数据模型修订号，用于数据库一致性检验。

**当前缺失项**：当前代码库中**尚未捕获任何构建时间戳（Build Time）**。

#### 3.1.2 构建时间戳（Build Time）捕获与 Cargo Crate 隔离穿透方案
为支持构建时间的准确展示，同时保证开源可重现构建（Reproducible Builds）的合规性与 Rust 增量编译性能，设计如下严密的跨 Crate 构建期捕获机制：

##### 3.1.2.1 Cargo Crate 隔离陷阱与根因分析
在 Cargo 工作区（Workspace）体系中，存在严格的 Crate 作用域隔离限制：
- **`cargo:rustc-env` 的作用域仅限于当前 Crate**：若在根 Crate `cyberapi-gui`（即根目录 `./build.rs`）中执行 `println!("cargo:rustc-env=CYBERAPI_BUILD_TIME=...")`，该环境变量**绝对无法**被依赖子 Crate `crates/cyberapi-core` 感知，在 `cyberapi-core` 中调用 `option_env!("CYBERAPI_BUILD_TIME")` 将永远返回 `None`。
- **根 `build-dependencies` 隔离**：根目录 `Cargo.toml` 中仅包含 `winresource`，子 Crate 不共享其构建依赖。
- **增量编译破坏陷阱**：若在 build 脚本中无条件每次生成当前时间且未配置 `cargo:rerun-if-env-changed`，Cargo 会认为 build 脚本每次都在产生新的构建环境，导致 `cyberapi-core` 及其所有下游依赖在每次 `cargo build` 时被强制全量重新编译！

##### 3.1.2.2 解决方案：`cyberapi-core` 专属 `build.rs` 与三级时间注入
1. **为 `crates/cyberapi-core` 创建专属 `build.rs`**：
   在 `crates/cyberapi-core/Cargo.toml` 中声明构建依赖：
   ```toml
   [build-dependencies]
   chrono = { workspace = true }
   ```
2. **`crates/cyberapi-core/build.rs` 增量友好实现**：
   ```rust
   // crates/cyberapi-core/build.rs
   fn main() {
       // 关键配置：只有当环境变量发生变化时才重新触发 build 脚本，保护本地日常增量编译秒级完成！
       println!("cargo:rerun-if-env-changed=CYBERAPI_BUILD_TIME");
       println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

       // 三级优先级获取构建时间：
       // 1. CI 或外部显式注入的环境变量 CYBERAPI_BUILD_TIME
       // 2. Linux / Nix 可重现构建标准变量 SOURCE_DATE_EPOCH
       // 3. 本地编译兜底：当前 UTC 时间
       let build_time = if let Ok(time) = std::env::var("CYBERAPI_BUILD_TIME") {
           time
       } else if let Ok(source_date) = std::env::var("SOURCE_DATE_EPOCH") {
           if let Ok(secs) = source_date.parse::<i64>() {
               chrono::DateTime::from_timestamp(secs, 0)
                   .map(|dt| dt.to_rfc3339())
                   .unwrap_or_else(|| chrono::Utc::now().to_rfc3339())
           } else {
               chrono::Utc::now().to_rfc3339()
           }
       } else {
           chrono::Utc::now().to_rfc3339()
       };

       println!("cargo:rustc-env=CYBERAPI_BUILD_TIME={build_time}");
   }
   ```
3. **CI 打包流水线注入 (`.github/workflows/package.yml`)**：
   在 Release / Nightly 打包步骤中，在运行 `cargo build` / `cargo-bundle` 之前将时间戳与 Commit 哈希写入 `$GITHUB_ENV`：
   ```yaml
   - name: Record version, commit and build time
     run: |
       echo "CYBERAPI_COMMIT=$(git rev-parse --short=7 HEAD)" >> "$GITHUB_ENV"
       echo "CYBERAPI_BUILD_TIME=$(date -u +"%Y-%m-%dT%H:%M:%SZ")" >> "$GITHUB_ENV"
   ```
4. **Core 层安全读取**：
   在 `crates/cyberapi-core/src/version.rs` 中：
   ```rust
   pub const BUILD_TIME: Option<&str> = match option_env!("CYBERAPI_BUILD_TIME") {
       Some(time) if !time.is_empty() => Some(time),
       _ => None,
   };
   ```

### 3.2 唤起入口与全局导航路由

为满足桌面端的人机工效学，独立的关于对话框需在以下三个入口无缝提供：

```
┌────────────────────────────────────────────────────────┐
│ 入口 1: macOS 原生应用菜单 (CyberAPI -> About CyberAPI) │
├────────────────────────────────────────────────────────┤
│ 入口 2: 标题栏设置下拉齿轮菜单 (Settings Gear -> 关于)  │
├────────────────────────────────────────────────────────┤
│ 入口 3: 全局设置窗口 (Settings -> Section::About)      │
└────────────────────────────────────────────────────────┘
```

#### 3.2.1 入口 1：macOS 原生菜单栏绑定
在 `src/lib.rs` 的 `cx.set_menus` 中声明全局动作并绑定至主应用菜单项：
```rust
actions!(app, [OpenAbout]);

// src/lib.rs 菜单配置
Menu::new(BUNDLE_NAME).items([
    MenuItem::action(format!("About {APP_NAME}"), OpenAbout),
    MenuItem::separator(),
    MenuItem::action(format!("Hide {BUNDLE_NAME}"), Hide),
    MenuItem::action("Hide Others", HideOthers),
    MenuItem::action("Show All", ShowAll),
    MenuItem::separator(),
    MenuItem::action(format!("Quit {BUNDLE_NAME}"), Quit),
])
```
通过全局动作监听器触发弹窗挂载：
```rust
cx.on_action(|_: &OpenAbout, cx| {
    on_window(cx, |window| {
        open_about_dialog(window, cx);
    });
});
```

#### 3.2.2 入口 2：标题栏设置下拉齿轮菜单
在 `src/root.rs`（第 490–515 行）中的 `render_menu(cx)` 追加菜单项：
```rust
let menu = menu
    .separator()
    .item(
        PopupMenuItem::new(t!("about.title").to_string())
            .icon(Icons::Info)
            .on_click(|_, window, cx| {
                open_about_dialog(window, cx);
            }),
    )
    .item(
        PopupMenuItem::new(t!("settings.title").to_string())
        // ...
    );
```

#### 3.2.3 入口 3：全局设置窗口关于分节与 Web 模态叠加冲突避让 (Web Modal Stacking Fix)
在 `src/views/settings.rs` 中，已经维护了 `Section::About`。但在 Web 浏览器环境下存在关键架构约束：
- **Web 环境弹窗现状**：在 Wasm 网页版中，设置界面本身就是一个通过 `window.open_dialog` 打开的顶层模态弹窗（`src/views/settings.rs:76`，此时 `window.has_active_dialog(cx)` 返回 `true`）。
- **多层弹窗死锁陷阱**：
  若在 `Section::About` 中尝试调用 `open_about_dialog(window, cx)`：
  1. `open_about_dialog` 包含守卫 `if window.has_active_dialog(cx) { return; }`，导致用户点击“查看关于”按钮时完全无响应（静默失败）。
  2. 若强行调用 `window.open_dialog`，根据 `AGENTS.md` 的规范警示：*“在弹窗上再叠一个弹窗或警告框，网页版不会画出来（但已打开），导致用户被隐藏的遮罩层锁定”*。
- **优雅的内联渲染方案 (Inline Rendering Solution)**：
  在 `src/views/settings.rs` 渲染 `Section::About` 时：
  - **直接内联渲染全部元信息**：将关于界面的所有核心要素（图标、应用名、Slogan、Version、Commit、Build Time、架构与 OS 内核、复制按钮、GitHub 链接）直接作为 `Section::About` 的视图内容渲染在设置面板右侧！
  - **双端无缝适配**：
    - 在 Web 端：用户在设置菜单切换至“关于”标签页，直接在设置容器内部完整查看和复制元信息，零弹窗叠加，零锁死风险。
    - 在桌面端：`Section::About` 同样内联展示基本信息，并提供“在新窗口/弹窗中查看”按钮，供用户按需弹出独立的关于对话框。
  - **标题栏/菜单唤起容错**：若用户在 Web 端已打开设置弹窗的情况下，又从标题栏下拉菜单点击“关于 CyberAPI”，系统不新建弹窗，而是直接调度已有设置实体的路由切换至 `Section::About`：`settings_view.update(cx, |s, cx| s.set_section(Section::About, cx))`。

### 3.3 UI 视觉结构与组件层级规范

独立的关于对话框遵循 GPUI 极简而现代的卡片设计规范：

```
┌──────────────────────────────────────────────────────────┐
│                      [X 顶部关闭]                        │
│                                                          │
│                     ┌──────────┐                         │
│                     │ 64x64 图标│                         │
│                     └──────────┘                         │
│                      CyberAPI                            │
│           现代化跨平台高生产力 HTTP/API 客户端            │
│                                                          │
│  ┌────────────────────────────────────────────────────┐  │
│  │ 版本 (Version)     0.3.0                           │  │
│  │ 提交 (Git Commit)  7a8581f  [📋 复制]              │  │
│  │ 构建时间 (Built)   2026-11-12T14:30:00Z            │  │
│  │ 目标平台 (Target)  macos / aarch64 (Apple Silicon) │  │
│  │ 系统内核 (OS)      Darwin 24.1.0 (macOS 15.1)      │  │
│  │ 数据模型 (Model)   Rev 24                          │  │
│  └────────────────────────────────────────────────────┘  │
│                                                          │
│       [ 检查更新 (Button) ]     [ GitHub 开源仓库 ↗ ]     │
│                                                          │
│         Copyright © 2026 CyberAPI Contributors           │
│              Released under Apache-2.0 / MIT             │
└──────────────────────────────────────────────────────────┘
```

#### 3.3.1 核心视觉与交互参数
- **模态宽度**：`px(440.)`，纵向流式自适应。
- **图标渲染**：通过 `img(ImageSource::Image(...))` 渲染 App 高清图标（`resources/icons/icon.png`），施加 `rounded_2xl` 圆角与温和阴影。
- **Commit 单击复制**：提供内联小按钮，点击后调用 `cx.write_to_clipboard(ClipboardItem::new_string(commit))`，并在 1.5 秒内将图标临时切换为 `Icons::Check` 并显示 Tooltip：“已复制到剪贴板”。
- **“检查更新”快捷动作**：直接触发后台检查并联动 R2 的标题栏状态机。

### 3.4 模态对话框构建模式与平台兼容性

1. **唯一性防重复保护与跨平台弹窗调度**：
   在打开弹窗前检测窗口状态：
   ```rust
   pub fn open_about_dialog(window: &mut Window, cx: &mut App) {
       // 防重复打开保护：若当前已有弹窗处于激活态
       if window.has_active_dialog(cx) {
           #[cfg(target_family = "wasm")]
           {
               // Web 环境容错：若用户在已打开 Settings 弹窗的情况下触发了 About
               // 不进行静默抛弃，而是关闭已有弹窗后唤起 About 弹窗，或直接重定向内部路由
               window.close_dialog(cx);
           }
           #[cfg(not(target_family = "wasm"))]
           {
               return;
           }
       }
       window.open_dialog(cx, |dialog, _, cx| {
           dialog
               .title(t!("about.title"))
               .w(px(440.))
               .keyboard(true)
               .overlay_closable(true)
               .close_button(true)
               .child(render_about_content(cx))
       });
   }
   ```
2. **GPUI 网页端多重弹窗限制与死锁防范 (Anti-Lockout Contract)**：
   在 `AGENTS.md` 中明确指出：*“在弹窗上再叠一个弹窗或警告框，网页版不会画出来”*。
   - **绝对禁止弹窗嵌套**：关于对话框作为独立一级弹窗打开时，其内部的所有交互操作（例如点击“检查更新”、点击“复制 Commit”）一律采用原位状态变化（如按钮图标变为对勾并显示微型 Tooltip）、或调用 `window.push_notification(Toast)` 进行无阻塞反馈，**严禁再次调用 `open_alert_dialog` 或 `open_dialog` 触发二级弹窗**。
   - **设置页内联分流**：在 Web 端进入 `设置 (Settings)` 弹窗后，如 §3.2.3 所述，`Section::About` 必须采用**纯内联渲染（Inline View）**，杜绝从弹窗内部再次唤起任何模态层，确保 Web 用户在任何路径下都不会遭遇界面死锁。

---

## 4. 需求 R4：响应体格式化默认美化方案 (Response Body Default Pretty Formatting)

### 4.1 历史背景与性能瓶颈深度复盘

#### 4.1.1 为什么历史版本曾回退为 Raw？
在 CyberAPI 的早期版本（commit `cf2d908`，Issue #51）中，系统曾将默认响应视图设为 `Raw` 模式。
**根本诱因在于 GPUI 底层的软折行算法瓶颈**：
- 当服务端返回单行体积达到数兆字节（如 5MB+）的未经任何换行的压缩 JSON（Minified JSON）时，GPUI 的排版引擎尝试在单一行上进行软折行（Soft-wrap）计算。
- 软折行计算复杂度为超长行的字符数级；单个超长行耗费数秒 CPU 算力，直接导致主 UI 线程挂起冻结。为防止无换行大文本崩溃，当时采取了保守策略，默认回退为不进行任何处理的 `Raw` 模式。

#### 4.1.2 瓶颈的彻底解决：PERF-3 与行折行截断
在后序架构优化中，团队在 `crates/cyberapi-core/src/format.rs` 引入了两项突破性技术：
1. **`format::json_prefix` 渐进式流式格式化算法 (PERF-3)**：
   - 彻底摆脱先做 `serde_json::from_slice` 构建完整内存语法树（AST）再做序列化的重型流程。
   - 采用纯字节状态机扫描，仅对前 `display_limit`（1MB 或 2MB）字节在单次遍历中直接插入换行符与缩进空格（`JSON_INDENT = 4`）。
   - 算法时间复杂度被严格约束在 $O(\text{limit})$，不论原始 Body 是 10MB 还是 100MB，流式格式化耗时始终稳定在 **1~2ms 以内**。
2. **`unwrappable_line` 软折行行长熔断 (`WRAP_LINE_LIMIT = 64KB`)**：
   - 系统为编辑器软折行设置了 64KB 的安全阈值。一旦某一行长度超出 64KB，排版引擎强制关闭该行的折行功能并退化为水平滚动，彻底杜绝排版挂起。

#### 4.1.3 核心逆转：Pretty 模式现在比 Raw 模式更流畅！
这是一个至关重要的性能结论：
- **Raw 模式下的风险**：当响应为超大压缩 JSON 时，Raw 模式由于没有任何换行，极易触发 `WRAP_LINE_LIMIT` 熔断，强制关闭软折行并导致水平滚动条无限拉长，阅读体验极差。
- **Pretty 模式下的优势**：在 Pretty 美化排版下，每个键值对与括号后都被自动插入了换行符，单行字符数几乎都在 80 字符以内。短行排版在 GPUI 中拥有极高的缓存命中率和极低的布局开销。
- **结论：默认启用 Pretty 美化模式不仅交互体验大幅改善，且在现代架构下性能与稳定性完全优于 Raw 模式**。

### 4.2 默认模式变更机制与数据流分析

#### 4.2.1 `JsonView` 枚举定义变更
在 `src/json_view_local.rs` 中：
```rust
// 修改前：
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JsonView {
    #[default]
    Raw,
    Pretty,
    Tree,
}

// 修改后：
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JsonView {
    Raw,
    #[default]
    Pretty,
    Tree,
}
```

#### 4.2.2 存储缺失回退升级
在读取本地存储配置 `src/json_view_local.rs::get(scope: &str)` 时：
```rust
pub fn get(scope: &str) -> JsonView {
    let Some(file) = storage::read_document() else {
        return JsonView::Pretty; // 从原来的 JsonView::Raw 改为 JsonView::Pretty
    };
    file.readers
        .get(scope)
        .and_then(|name| JsonView::parse(name))
        .unwrap_or_default() // 此处自动解析为 Pretty
}
```

#### 4.2.3 切换按钮组展示顺序重构
在 `src/json_view_local.rs` 中将 `JsonView::ALL` 常量调整为：
```rust
// 修改前：pub const ALL: [Self; 3] = [Self::Raw, Self::Pretty, Self::Tree];
// 修改后：
pub const ALL: [Self; 3] = [Self::Pretty, Self::Raw, Self::Tree];
```
- **视觉契合**：响应面板右上角的切换按钮栏渲染顺序变为 `[ 美化 | 原始 | 树 ]`，默认选中的“美化”位于首位。
- **与主流工具看齐**：Postman、Insomnia 等行业工具均将 Pretty 置于首位，符合全球开发者操作直觉。
- **兼容性**：由于序列化与解析均采用字符串关键字（`"pretty"`, `"raw"`, `"tree"`），数组顺序调整对底层数据完全无感。

### 4.3 与 JSONPath 过滤功能的深度协同

在当前 `src/views/workspace/response_panel.rs` 中，JSONPath 交互搜索栏的启用条件被显式限制：
```rust
// 仅当非 Raw 模式时允许输入 JSONPath
if response_view != JsonView::Raw {
    // 渲染 JSONPath 过滤输入框与匹配计数器
}
```
- 在旧版本默认 Raw 模式下，用户每次拿到响应，JSONPath 过滤框均处于禁用或隐匿状态，极大削弱了该特性的可用性。
- **升级为默认 Pretty 模式后，API 响应到达的瞬间，JSONPath 过滤引擎立即处于就绪状态**，用户可无缝通过快捷键激活并直接键入 `$.data.items[*].id` 进行实时数据探查。

### 4.4 用户手动切换与记忆机制设计

系统维护了严密的“全局默认 vs 会话覆盖”两级记忆模型：

```
┌────────────────────────────────────────────────────────┐
│                   Global Preference                    │
│   Desktop: json-view.json / Web: localStorage          │
│   生效范围: 所有全新打开的接口与新生成的响应           │
├────────────────────────────────────────────────────────┤
│                   Response Override                    │
│   In-Memory: ResponsePanel.body_views                  │
│   生效范围: 当前标签页会话内针对特定历史响应的手动调整  │
└────────────────────────────────────────────────────────┘
```

1. **响应级临时覆盖（Response Override）**：
   - 用户在当前打开的响应面板中手动将视图切至 `Raw`。
   - 现有的 `body_view: Option<(String, JsonView)>` 绑定了当前响应的 ID。
   - 为避免在多条历史记录之间切换时丢失手动选择，将其演进为轻量内存字典：`body_views: HashMap<String, JsonView>`。
   - 当用户在历史响应 A 与响应 B 之间切换时，分别保留各自的手动切换状态。
2. **全局首选项（Global Preference）**：
   - 若用户期望永久以 Raw 作为默认模式，可在 `设置 -> 编辑器 -> JSON 默认显示` 下将首选视图设置为 Raw。
   - 设置项持久化于本地配置，后续所有新请求将尊重此全局设定。

### 4.5 极限边界、容错与多级降级安全网设计 (Exhaustive Safeguards & Fallbacks)

为保证在各种极端、恶意或海量数据场景下响应面板绝对不崩溃、不卡死、不高内存占用，系统构建了五重端到端防御安全网：

```
┌────────────────────────────────────────────────────────────────────────┐
│                        HTTP Response Payload                           │
└──────────────────────────────────┬─────────────────────────────────────┘
                                   │
      ┌────────────────────────────┼────────────────────────────┐
      ▼                            ▼                            ▼
[非文本/二进制/图像]       [Server-Sent Events / WS]        [JSON / 文本响应]
  HEX_LIMIT: 256KB           流式累积, 动态表格路由        Pretty 默认美化排版
  HexDump / 图像预览         零静态格式化阻塞                    │
                                                                ▼
                                                   ┌─────────────────────────┐
                                                   │ 体积 > display_limit?   │
                                                   └────┬───────────────┬────┘
                                                   Yes  │               │ No
                                                        ▼               ▼
                                                  json_prefix       pretty_json
                                                  O(limit) 截断    完整 AST 缩进
                                                        │               │
                                                        └───────┬───────┘
                                                                │
                                                                ▼
                                                   ┌─────────────────────────┐
                                                   │ 单行 > WRAP_LINE_LIMIT? │
                                                   └────┬───────────────┬────┘
                                                   Yes  │               │ No
                                                        ▼               ▼
                                                   熔断软折行      启用平滑软折行
                                                   强制水平滚动    (60fps 排版)
```

#### 4.5.1 边界 1：Content-Type 与媒体类型类型安全分流
1. **JSON 类型 (`BodyKind::Json`)**：
   - 当服务端返回 `Content-Type: application/json` 或以 `+json` 结尾的媒体类型时，系统认定为 JSON，默认启用 Pretty 格式化通道。
2. **纯文本与结构化标记 (`BodyKind::Text`)**：
   - 对于 `text/plain`, `text/html`, `text/csv`, `text/xml`, `text/css`，系统直接以原生文本流（UTF-8）展示。
   - 切换按钮栏自动隐藏 `Pretty` 和 `Tree` 选项，仅保留 `Raw` 模式与语法高亮（HTML/XML/CSS），防止无谓的 JSON 语法尝试。
3. **图像与多媒体分流 (`BodyKind::Image`)**：
   - **矢量图像 (SVG)**：支持代码模式与预览模式双向切换。代码模式下通过 `text_prefix` 流式截断保护。
   - **位图图像 (PNG, JPEG, WebP, GIF, ICO)**：绕过文本编辑器，自动路由至 GPUI 图像渲染组件，提供 `[ 适应窗口 | 100% | 放大 | 缩小 ]` 交互。
4. **二进制与 PDF 兜底 (`BodyKind::Pdf | BodyKind::Binary`)**：
   - 绝不尝试将不可打印的二进制字节作为字符串强转。
   - 自动切入十六进制转储器（Hex Dump View），严格执行 `HEX_LIMIT = 256KB`（`src/views/workspace/response_panel.rs:2321`）。仅显示前 256KB 的内存偏移量、Hex 字节与 ASCII 预览列，并提示“已截断，完整二进制文件可通过保存到磁盘查看”。

#### 4.5.2 边界 2：超大 Payload 与流式前缀熔断 (`json_prefix` 与 `TREE_LIMIT`)
针对体积达数兆甚至数十兆的巨型响应，设立双层熔断红线：
1. **编辑器展示截断 (`display_limit`，默认 1 MB / 可配至 16 MB)**：
   - 当 `response.body.len() > display_limit` 时，系统严禁调用 `serde_json::from_slice` 构造完整 AST，而是自动切换至 `cyberapi_core::format::json_prefix(&response.body, limit)`（PERF-3）。
   - **算法保证**：`json_prefix` 仅顺序读取前 `limit` 字节，通过纯字节自动机单遍扫描（Single-pass Scanning），在不分配整树内存的情况下直接输出格式化前缀，耗时恒定在 1~2ms 以内。
   - **UI 呈现**：在编辑器顶部挂载醒目的警告胶囊：“响应体已超过显示上限（1 MB），已截断展示前缀；您可通过右上角‘保存’按钮导出完整数据”。
2. **JSON 树状视图熔断 (`TREE_LIMIT = 16 MB`)**：
   - 树状视图（Tree View）依赖构建嵌套节点树，若对超过 16MB 的 JSON 生成节点对象将导致剧烈的内存分配颠簸（GC/OOM 隐患）。
   - 代码级强约束（`response_panel.rs:546`）：一旦 `response.body.len() > TREE_LIMIT`，强制将 Tree 模式置为不可用，展示提示卡片：“响应数据过大（> 16 MB），树状视图已停用以保护内存”，并引导用户在 Pretty 截断文本中探查。

#### 4.5.3 边界 3：软折行单行行长熔断 (`WRAP_LINE_LIMIT = 64 KB`)
GPUI 的文本排版引擎在计算文字折行时，对长度极端的单行（例如未经压缩的 100KB 单行 Base64 字符串）具有显著的排版开销。
- 系统在每次文本装配时调用 `cyberapi_core::response::unwrappable_line(&text)`。
- **熔断判定**：若检测到格式化后的文本中存在任何一行字符数超过 `WRAP_LINE_LIMIT = 65,536`（64 KB）：
  ```rust
  self.long_line = unwrappable_line(&text);
  let wrap = self.wrap && self.long_line.is_none() && !hex;
  ```
- **降级行为**：立即对当前编辑器关闭软折行（`soft_wrap(false)`），退化为平滑横向滚动模式（Horizontal Scrollbar），彻底杜绝超长行排版卡死主 UI 线程的隐患。

#### 4.5.4 边界 4：流式传输与分块响应保护 (SSE & Chunked Streams)
当遇到 Server-Sent Events（`Content-Type: text/event-stream`）或 WebSocket 持续帧传输时：
- 系统不将其作为静态单次 Body 处理，而是通过 `api.live()` 与 `self.events` 动态桥接。
- 接收到的事件按条目逐行填充进入结构化流式事件表格（Event Stream Table），支持“停止”接收并存留已有事件。静态美化排版与 JSONPath 过滤自动避让，绝不因长连接持续推流引发排版重入。

#### 4.5.5 边界 5：畸形 / 截断 JSON 优雅回退与零 Panic 保障
当服务端声明了 `application/json`，但实际响应体因连接意外中断导致 JSON 截断（例如在对象中间关闭连接），或包含了非法控制字符时：
- **零 Panic 原则**：源码底层 `pretty_json` 实现采用安全模式匹配：
  ```rust
  pub fn body_text(&self) -> Option<String> {
      match self.body_kind() {
          BodyKind::Json => {
              Some(pretty_json(&self.body).unwrap_or_else(|| String::from_utf8_lossy(&self.body).into()))
          }
          // ...
      }
  }
  ```
- **平滑降级表现**：若 `pretty_json` 解析失败，直接降级为 lossy UTF-8 原文展示，绝不调用任何 `unwrap()` 或抛出未处理异常。
- **高亮容错**：编辑器内集成的 Tree-sitter 语法分析器具备天然的局部容错能力（Error Tolerant Parsing），能够正确高亮语法出错点之前的所有键值对，并将出错节点局部标红，保证开发者清晰直观地发现服务端返回数据的断流或格式缺陷。

---

## 5. 需求 R5：URL QueryString 模式可选与兼容存储方案 (Configurable URL QueryString Mode & Storage Compatibility)

### 5.1 需求定位：设置项驱动的双模式共存

#### 5.1.1 核心诉求与设计目标
不同开发者对 Query 参数的编辑习惯存在差异：
- **表格编辑模式（默认，当前行为）**：在 URL 输入框中输入或粘贴 `?page=1&limit=20` 时，失焦/回车后由 `take_query` 提取并合入下方独立的 `Query` 表格编辑器中，方便逐行勾选启用/禁用、编写参数说明、查看 OpenAPI 类型约束或切换批量文本编辑。
- **URL 内联编辑模式（可选，设置中开启）**：部分用户更习惯直接在 URL 输入框中保留和修改完整的 `?page=1&limit=20`，此时**不使用单独的 Query 参数表格编辑器**，URL 输入框失焦或回车时不剥离 QueryString。
- **底层保存双向兼容**：无论是同一个用户在设置中随时切换这两种模式，还是团队协作中不同成员各自使用不同模式编辑同一个项目，**保存落盘的数据结构必须 100% 兼容互通**，不丢失参数、不重复拼接、不破坏现有的发送管线与导入导出。

---

### 5.2 偏好设置模型与设置界面设计 (`TextPrefs.query_in_url`)

#### 5.2.1 `TextPrefs` 新增 `query_in_url` 字段
在 `crates/cyberapi-core/src/prefs.rs` 的 `TextPrefs`（对应“设置 -> 编辑器”面板）中增加布尔开关 `query_in_url`，**默认值为 `false`（保持当前表格编辑模式）**：

```rust
// crates/cyberapi-core/src/prefs.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TextPrefs {
    /// The interface's: the rem everything is measured in.
    pub ui_size: u32,
    /// The code editors': a request's body and a response's.
    pub editor_size: u32,
    pub wrap: bool,
    /// The open APIs above the request and the response.
    pub show_tabs: bool,
    /// R5: whether query parameters are edited directly inside the URL input
    /// instead of the standalone Query key-value table editor.
    /// Defaults to `false` (standalone Query table editor mode).
    #[serde(default)]
    pub query_in_url: bool,
}

impl Default for TextPrefs {
    fn default() -> Self {
        Self {
            ui_size: 0,
            editor_size: 0,
            wrap: true,
            show_tabs: true,
            query_in_url: false, // 默认保持当前表格编辑模式
        }
    }
}
```
- 由于 `TextPrefs` 标注了 `#[serde(default)]`，旧版配置文件反序列化时 `query_in_url` 自动为 `false`，零迁移成本。

#### 5.2.2 设置面板（`src/views/settings.rs`）入口
在 `设置 -> 编辑器 (Editor)` 页签中（紧随“显示打开接口的标签栏 `show_tabs`”开关下方），新增一行开关配置：
- **标签文案**：`在 URL 中直接编辑 Query 参数` / `Edit query parameters in URL`
- **说明文案**：`开启后，Query 参数直接保留在地址栏中编辑，不再使用独立的 Query 表格编辑器`
- **切换即时生效**：当用户在设置中切换该开关时，触发 `AppState` 保存 `prefs` 并通知当前打开的 `RequestEditor` 刷新显示形态（见 §5.6.1）。

---

### 5.3 两种模式下的编辑器 UI 与 Tab 栏行为

| 维度 | 模式 A：表格编辑模式（`query_in_url = false`，默认） | 模式 B：URL 直编模式（`query_in_url = true`） |
|---|---|---|
| **URL 输入框显示** | 仅显示不含 `?query` 的基础路径（如 `/v1/items`） | 将 `HttpRequest.uri` 与已启用的 `HttpRequest.query` 合成完整带参 URL 显示（如 `/v1/items?page=1&limit=20`） |
| **失焦 / 回车行为** | 触发 `take_query(window, cx)`，自动剥离 `?query` 并追加到 `self.query` 表格 | **不执行 `take_query`**，`?query` 原样保留在 URL 输入框中，直接调用 `save_uri(cx)` |
| **粘贴 cURL (`take_curl`)** | 解析出的 query 参数填入 `self.query` 表格，URL 栏仅放基础路径 | 解析出的完整带参 URL 直接填入 URL 输入框，同时同步回底层 `HttpRequest` |
| **独立 Query 表格 (`self.query`)** | 正常渲染在 `Query` 选项卡中 | **不渲染独立的 Query 表格编辑器** |
| **路径参数 (`:param` / `{param}`)** | 当 URL 含路径占位符时，渲染在 `Query` 选项卡上方 | 当 URL 含路径占位符时，`Query` 选项卡仅展示路径参数表 (`self.path_params`)；若无路径占位符，隐藏 `Query` 选项卡（若当前正处于 `Query` Tab，GET 请求自动回退选中 `Headers`，POST 等有 Body 请求选中 `Body`） |

---

### 5.4 模板感知型 URL 解析与组装机制 (`split_uri_query` / `split_query_pairs`)

#### 5.4.1 为什么不能直接用标准 `url::Url` 解析编辑器文本？
在 API 客户端中，URL 输入框经常包含环境变量前缀或模板函数（例如 `{{HOST}}/api/search?q={{keyword}}&filter={{get("tag", "a&b=c")}}&expr={{calc(1+1)}}#section`）：
1. `url::Url::parse` 会因 `{{HOST}}` 报错 `InvalidDomainCharacter`，或因相对路径 `/api/search` 报错 `RelativeUrlWithoutBase`。
2. `url::form_urlencoded` 会将 `{` / `}` 百分号转义为 `%7B` / `%7D` 破坏模板语法，并将 `+` 转为空格破坏 `{{calc(1+1)}}`。
3. 朴素的 `split('?')` / `split('&')` / `split('=')` 会把模板内部（如 `{{get("tag", "a&b=c")}}`）的 `&` 和 `=` 误判为分隔符。

#### 5.4.2 模板感知的 URI 分割器 (`split_uri_query`)
在 `crates/cyberapi-core/src/import/mod.rs`（或 `uri.rs`）中提供模板感知的 URI 切分函数，严格遵循 RFC 3986 §3.5（`?query` 在 `#fragment` 之前）并保护 `{{ ... }}` 内部字符：

```rust
/// 模板感知的 URI 分割器：将 URI 切分为 (base_path, Option<query_string>, Option<fragment>)
/// 严格保护 {{ ... }} 内部出现的一切 '?' 或 '#'，不将其作为 URL 定界符误判。
pub fn split_uri_query(uri: &str) -> (&str, Option<&str>, Option<&str>) {
    let mut in_template = 0usize;
    let mut chars = uri.char_indices().peekable();
    let mut query_start = None;
    let mut fragment_start = None;

    while let Some((i, ch)) = chars.next() {
        if ch == '{' && chars.peek().map(|&(_, c)| c) == Some('{') {
            chars.next();
            in_template = in_template.saturating_add(1);
            continue;
        }
        if ch == '}' && chars.peek().map(|&(_, c)| c) == Some('}') {
            chars.next();
            in_template = in_template.saturating_sub(1);
            continue;
        }

        if in_template == 0 {
            if ch == '?' && query_start.is_none() && fragment_start.is_none() {
                query_start = Some(i);
            } else if ch == '#' && fragment_start.is_none() {
                fragment_start = Some(i);
            }
        }
    }

    match (query_start, fragment_start) {
        (Some(q), Some(f)) => (&uri[..q], Some(&uri[q + 1..f]), Some(&uri[f + 1..])),
        (Some(q), None) => (&uri[..q], Some(&uri[q + 1..]), None),
        (None, Some(f)) => (&uri[..f], None, Some(&uri[f + 1..])),
        (None, None) => (uri, None, None),
    }
}
```

#### 5.4.3 模板感知型查询参数切分器 (`split_query_pairs`)
```rust
/// 将 query_str 解析为 [(key, value)]，严格保护 {{ ... }} 内部出现的 '&' 和 '='
pub fn split_query_pairs(query: &str) -> Vec<(&str, &str)> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let mut pairs = Vec::new();
    let mut in_template = 0usize;
    let mut item_start = 0usize;
    let mut eq_pos = None;
    let mut chars = query.char_indices().peekable();

    while let Some((i, ch)) = chars.next() {
        if ch == '{' && chars.peek().map(|&(_, c)| c) == Some('{') {
            chars.next();
            in_template = in_template.saturating_add(1);
            continue;
        }
        if ch == '}' && chars.peek().map(|&(_, c)| c) == Some('}') {
            chars.next();
            in_template = in_template.saturating_sub(1);
            continue;
        }

        if in_template == 0 {
            if ch == '=' && eq_pos.is_none() {
                eq_pos = Some(i);
            } else if ch == '&' {
                let raw_item = &query[item_start..i];
                if !raw_item.is_empty() {
                    match eq_pos {
                        Some(eq) if eq >= item_start => {
                            pairs.push((&query[item_start..eq], &query[eq + 1..i]));
                        }
                        _ => pairs.push((raw_item, "")),
                    }
                }
                item_start = i + 1;
                eq_pos = None;
            }
        }
    }

    if item_start < query.len() {
        let raw_item = &query[item_start..];
        match eq_pos {
            Some(eq) if eq >= item_start => {
                pairs.push((&query[item_start..eq], &query[eq + 1..]));
            }
            _ => pairs.push((raw_item, "")),
        }
    }

    pairs
}
```

---

### 5.5 保存与加载双向兼容设计（单一底层存储结构）

#### 5.5.1 核心原则：保持持久化模型 `HttpRequest { uri, query }` 100% 统一
为了确保：
1. 开启 `query_in_url = true` 的用户保存请求后，使用默认表格模式（`query_in_url = false`）的用户打开时能看到完整的 `Query` 表格；
2. 使用默认表格模式保存的请求，在 `query_in_url = true` 模式下打开时能直接在 URL 输入框中看到完整的 `?k1=v1&k2=v2`；
3. 现有的发送引擎（`send.rs`）、Postman/OpenAPI 导入导出、在线接口文档、MCP 工具无需改变数据契约；

**持久化层的 `HttpRequest` 保持统一的规范结构**：
- **`HttpRequest.uri`**：始终保存**不含 `?query` 的基础 URI**（若含 `#fragment` 则保留在末尾，如 `{{HOST}}/v1/search#section`）。
- **`HttpRequest.query: Vec<KvParam>`**：始终保存结构化的 Query 参数列表。

#### 5.5.2 在 `query_in_url = true` 模式下加载/展示 URL（Model -> UI）
当 `RequestEditor` 加载接口（或接收 `ApiEvent::RequestReplaced` / 切换偏好设置）且 `query_in_url == true` 时，通过辅助函数 `format_uri_with_query(&request.uri, &request.query)` 生成输入框展示文本：

```rust
/// 将规范化存储的 base_uri 与已启用的 query 参数合成为完整带参 URL（供 query_in_url 模式展示）
pub fn format_uri_with_query(uri: &str, query: &[KvParam]) -> String {
    let (base, existing_query, fragment) = split_uri_query(uri);
    let enabled_pairs: Vec<String> = query
        .iter()
        .filter(|kv| kv.enabled && !kv.key.trim().is_empty())
        .map(|kv| {
            if kv.value.is_empty() {
                kv.key.clone()
            } else {
                format!("{}={}", kv.key, kv.value)
            }
        })
        .collect();

    let combined_query = match (existing_query.filter(|q| !q.is_empty()), enabled_pairs.is_empty()) {
        (Some(eq), false) => Some(format!("{eq}&{}", enabled_pairs.join("&"))),
        (Some(eq), true) => Some(eq.to_string()),
        (None, false) => Some(enabled_pairs.join("&")),
        (None, true) => None,
    };

    let fragment_part = fragment.map(|f| format!("#{f}")).unwrap_or_default();
    match combined_query {
        Some(q) => format!("{base}?{q}{fragment_part}"),
        None => format!("{base}{fragment_part}"),
    }
}
```
随后再经过 `split_env_prefix` 剥离前导 `{{ENV}}` 按钮，将剩余部分（含 `?k1=v1&k2=v2#fragment`）填入 `self.uri` 输入框。

#### 5.5.3 在 `query_in_url = true` 模式下保存 URL（UI -> Model）
当用户在 `query_in_url = true` 模式下编辑 URL 输入框并触发保存（`save_uri`）时，将输入框中的完整 URL 无损拆解回规范化的 `(request.uri, request.query)`：

```rust
/// 在 query_in_url 模式下，将完整带参 URL 拆回规范化的 (clean_uri, updated_query)
/// 同时保留原有同名参数的 description / OpenAPI 元数据，以及原先未启用 (enabled: false) 的参数行。
pub fn parse_uri_into_query(full_uri: &str, previous_query: &[KvParam]) -> (String, Vec<KvParam>) {
    let (base, query_opt, fragment) = split_uri_query(full_uri);
    let clean_uri = match fragment {
        Some(f) => format!("{base}#{f}"),
        None => base.to_string(),
    };

    // 记录旧参数中每个同名 key 的元数据（说明、类型、是否必填等），按出现顺序消费
    let mut old_enabled: Vec<&KvParam> = previous_query.iter().filter(|kv| kv.enabled).collect();
    let mut new_query: Vec<KvParam> = Vec::new();

    if let Some(query_str) = query_opt {
        for (k, v) in split_query_pairs(query_str) {
            let mut param = KvParam::new(k, v);
            // 若旧列表中存在同名已启用参数，继承其字段说明与 OpenAPI schema 元数据
            if let Some(pos) = old_enabled.iter().position(|old| old.key == k) {
                let matched = old_enabled.remove(pos);
                param.description = matched.description.clone();
                param.param_type = matched.param_type.clone();
                param.required = matched.required;
                param.format = matched.format.clone();
                param.enum_values = matched.enum_values.clone();
            }
            new_query.push(param);
        }
    }

    // 保留原先在表格模式中临时勾选禁用的参数（enabled == false），避免切到 URL 模式编辑后再切回表格模式时丢失未启用项
    for disabled in previous_query.iter().filter(|kv| !kv.enabled && !kv.key.trim().is_empty()) {
        new_query.push(disabled.clone());
    }

    (clean_uri, new_query)
}
```

---

### 5.6 模式热切换、参数元数据保留与下游零侵入兼容

#### 5.6.1 设置切换时的实时双向同步
当用户在 `设置 -> 编辑器` 中切换 `query_in_url` 开关时：
1. **从表格模式切换到 URL 模式（`false -> true`）**：
   - `RequestEditor` 监听到 `AppState` 偏好变更，立即调用 `format_uri_with_query(&request.uri, &request.query)` 刷新 `self.uri` 输入框，将表格中所有已启用的参数直接展现在 URL 栏尾部，并隐藏独立的 Query 表格。
2. **从 URL 模式切换回表格模式（`true -> false`）**：
   - 先对当前 URL 输入框执行一次 `parse_uri_into_query` 同步至 `request.uri` 与 `request.query`；
   - 将 `self.uri` 输入框刷新为不含 `?query` 的 `request.uri`，并将 `self.query` 表格刷新为最新的 `request.query`，恢复展示 `Query` 选项卡。

#### 5.6.2 下游模块 100% 零侵入兼容
由于底层存储与内存中的 `HttpRequest` 始终维持 `uri`（不含 `?query`）+ `query: Vec<KvParam>` 的标准结构：
- **发送管线 (`crates/cyberapi-core/src/send.rs`)**：无需改动，继续对 `request.uri` 渲染路径参数、对 `request.query` 逐项求值 `{{模板}}` 并做百分号编码，天然支持 URL 模式下的模板求值！
- **旧版客户端与团队同步 (`ServerCopy`)**：绝对不会发生 `?p=1&p=1` 参数翻倍问题，因为落盘的 `request.uri` 不含冗余的 `?p=1`，所有参数均规范存储在 `request.query` 中。
- **导入导出 (`Postman` / `OpenAPI` / `cURL`)、在线文档 (`docs_editor.rs`) 与 MCP (`mcp.rs`)**：全部直接消费标准的 `request.uri` 与 `request.query`，并且通过 `parse_uri_into_query` 的元数据继承机制，OpenAPI 导入的字段说明（`description`）和类型约束在 URL 直编模式下编辑参数值时依然完好无损。

---

## 6. 实施路径与任务分解清单 (Implementation Roadmap & Checklist)

### 6.1 修改文件全景清单

根据“代码零侵入原则”，以下为后续编码实施阶段需要触达并修改的文件全景列表：

```
cyberapi-gpui/
├── .github/workflows/package.yml              # [R3] CI 构建环境注入 CYBERAPI_BUILD_TIME
├── crates/
│   └── cyberapi-core/
│       ├── Cargo.toml                         # [R3] 引入 build-dependencies: chrono = { workspace = true }
│       ├── build.rs (新建)                     # [R3] 专属构建脚本: 注入 CYBERAPI_BUILD_TIME, 增量编译守卫
│       └── src/
│           ├── version.rs                     # [R3] 增加 BUILD_TIME 常量定义
│           ├── import/mod.rs                  # [R5] 实现 split_uri_query、split_query_pairs、format_uri_with_query、parse_uri_into_query
│           └── prefs.rs                       # [R5] TextPrefs 新增 query_in_url: bool (默认 false)
└── src/
    ├── json_view_local.rs                     # [R4] JsonView 默认值切为 Pretty, 调整 ALL 顺序
    ├── states/
    │   ├── app.rs                             # [R4] reset_prefs 继承 Pretty
    │   └── update.rs                          # [R2] 导出更细粒度 Checking/Progress/Failed 状态流, 精确跳过匹配
    ├── update_install.rs                      # [R2] Windows 便携 Zip 打开 Explorer 交互对齐
    ├── root.rs                                # [R2, R3] 标题栏 update 视图挂载 (occlude/stop_propagation), 设置菜单加 About
    ├── lib.rs                                 # [R3] macOS set_menus 挂载 OpenAbout
    └── views/
        ├── about_dialog.rs (新建)              # [R3] 独立“关于”模态对话框视图实现 (防多重弹窗)
        ├── update_dialog.rs                   # [R2] 标题栏 update_chip 重构为 Popover + 原位进度条 + 失败重试
        ├── settings.rs                        # [R3, R4, R5] About 详情页、JSON 默认显示顺序、编辑器“在 URL 中直接编辑 Query 参数”开关
        └── workspace/
            ├── request_editor.rs              # [R5] 按 TextPrefs.query_in_url 切换表格模式 / URL 直编模式及保存双向同步
            └── response_panel.rs              # [R4] Pretty 默认生效确认, body_views 字典优化, 五重边界防护
```

### 6.2 详细测试用例规划

为保障高质量交付，每个模块均需配套严格的行为驱动测试（BDD）：

#### 模块 R2：标题栏更新测试集
- `test_titlebar_update_chip_phase_transitions`: 验证在 `Idle`, `Checking`, `Downloading`, `Installed`, `Failed` 各阶段，更新指示器生成的 GPUI 元素符合设计规范。
- `test_update_skip_exact_version_matching`: 验证在 Popover 中点击“跳过更新”后，`info.version` 精准写入 `AppPrefs.updates.skipped`；相同版本自动检查静默跳过，但新补丁版（如 `v0.3.1`）或手动检查依然能正常提示。
- `test_titlebar_update_occlude_prevents_window_drag`: 验证更新容器添加 `.occlude()` 与 `on_mouse_down(MouseButton::Left)` 的 `cx.stop_propagation()` 后，鼠标点击不会被 `TitleBar` 误判为窗口拖动。
- `test_inplace_download_failure_handling`: 验证在下载中断或校验失败分发 `UpdateEvent::InstallFailed` 时，标题栏原位变换为错误/重试胶囊，`follow_update` 弹出带有重试动作的全局 Toast 通知。
- `test_windows_handover_zip_package`: 验证在 Windows 平台下载完成后，调用 `explorer /select` 定位 Zip 包，不尝试执行未构建的 `.exe` 安装程序。

#### 模块 R3：独立关于对话框测试集
- `test_version_metadata_presence`: 断言 `VERSION`, `COMMIT`, `BUILD_TIME`, `MODEL` 均具有非空有效的格式，且 `cyberapi-core` 正确捕获自身构建脚本导出的环境变量。
- `test_about_dialog_single_instance`: 验证在已有激活对话框时再次触发 `OpenAbout`，不会发生多重叠加或 Web 端死锁。
- `test_web_settings_about_inline_rendering`: 验证在 Web 模式下打开 Settings 时，`Section::About` 采用纯内联渲染，不触发任何额外的模态对话框。

#### 模块 R4：响应格式化美化测试集
- `test_json_view_default_is_pretty`:
  ```rust
  #[test]
  fn test_json_view_default_is_pretty() {
      assert_eq!(JsonView::default(), JsonView::Pretty);
      assert_eq!(JsonView::ALL[0], JsonView::Pretty);
  }
  ```
- `test_new_response_opens_pretty`: 在 `TestAppContext` 下模拟收到 JSON 响应，断言默认处于 Pretty 模式且文本已被缩进。
- `test_huge_response_json_prefix_performance`: 构造 10MB 压缩 JSON，验证 Pretty 模式下的截断耗时在 5ms 以内，无内存飙升。
- `test_json_tree_limit_safeguard`: 构造超过 16MB 的巨型响应体，验证树状视图自动熔断并提示不可用，不分配整树节点内存。
- `test_wrap_line_limit_safeguard`: 构造单行超过 64KB 的文本，验证软折行自动熔断关闭，平滑退化为横向滚动。
- `test_malformed_json_fallback_lossy`: 构造语法破损的截断 JSON，验证 `pretty_json` 失败后优雅回退为 UTF-8 原始文本，零 panic。

#### 模块 R5：URL QueryString 双模式与兼容存储测试集
- `test_text_prefs_query_in_url_defaults_false`: 验证 `TextPrefs::default().query_in_url == false`，默认保持表格编辑模式。
- `test_split_uri_query_with_fragment_and_templates`:
  ```rust
  #[test]
  fn test_split_uri_query_with_fragment_and_templates() {
      let uri = "{{HOST}}/search?q={{query?default}}&limit=10#section";
      let (base, query, fragment) = split_uri_query(uri);
      assert_eq!(base, "{{HOST}}/search");
      assert_eq!(query, Some("q={{query?default}}&limit=10"));
      assert_eq!(fragment, Some("section"));
  }
  ```
- `test_split_query_pairs_preserves_templates`:
  ```rust
  #[test]
  fn test_split_query_pairs_preserves_templates() {
      let query = "filter={{get(\"tag\", \"a&b=c\")}}&limit={{1+1}}&sort=desc";
      let pairs = split_query_pairs(query);
      assert_eq!(pairs, vec![
          ("filter", "{{get(\"tag\", \"a&b=c\")}}"),
          ("limit", "{{1+1}}"),
          ("sort", "desc"),
      ]);
  }
  ```
- `test_format_uri_with_query_and_fragment`: 验证 `format_uri_with_query("https://api.com/search#top", &[KvParam::new("page", "1")])` 输出 `"https://api.com/search?page=1#top"`，禁用项（`enabled: false`）不拼入 URL。
- `test_parse_uri_into_query_preserves_metadata_and_disabled`: 验证在 URL 直编模式下修改 `?page=2` 保存时，规范化拆出 `clean_uri = "https://api.com/search"` 与 `query = [page=2]`，且原 `page` 参数上的 `description` / `param_type` / `required` 元数据及未启用的 `draft=true` 参数完整保留。
- `test_mode_switch_roundtrip_lossless`: 验证同一请求在 `query_in_url = false`（表格模式）与 `query_in_url = true`（URL 模式）之间来回切换并保存时，数据 100% 无损且不产生重复参数。

### 6.3 国际化与本地化文案字典变更

需在 `locales/zh-CN.toml` 与 `locales/en.toml` 同步新增/更新键值对：

```toml
# locales/zh-CN.toml
[about]
title = "关于 CyberAPI"
commit = "提交哈希"
build_time = "构建时间"
platform = "系统架构"
copy_commit = "复制 Commit"
copied = "已复制"
check_updates = "检查更新"
homepage = "官方主页"
license = "开源许可"

[update]
checking = "正在检查更新..."
title = "发现新版本 {version}"
versions = "当前: {current}  最新: {latest}"
skip = "跳过此版本"
update_and_restart = "更新并重启"
downloading = "正在下载更新"
ready_to_restart = "准备就绪，正在重启应用..."

[settings]
query_in_url = "在 URL 中直接编辑 Query 参数"
query_in_url_desc = "开启后，Query 参数直接保留在地址栏中编辑，不再使用独立的 Query 表格编辑器"
```

```toml
# locales/en.toml
[about]
title = "About CyberAPI"
commit = "Commit"
build_time = "Build Time"
platform = "Platform"
copy_commit = "Copy Commit"
copied = "Copied"
check_updates = "Check for Updates"
homepage = "Homepage"
license = "License"

[update]
checking = "Checking for updates..."
title = "New version available: {version}"
versions = "Current: {current}  Latest: {latest}"
skip = "Skip this version"
update_and_restart = "Update & Restart"
downloading = "Downloading update"
ready_to_restart = "Ready, restarting application..."

[settings]
query_in_url = "Edit query parameters in URL"
query_in_url_desc = "Keep query string parameters in the URL bar instead of using the standalone Query table editor"
```

### 6.4 面向用户文档同步规划

根据 `AGENTS.md` 规范，所有面对用户的功能变更必须以中英双语同步维护：
1. **`README.md` / `README_zh.md`**：
   - 响应面板功能亮点更新：JSON 响应默认美化呈现，开箱支持 JSONPath 过滤。
2. **`docs/FEATURES.md` / `docs/FEATURES_zh.md`**：
   - 补充“标题栏原位更新体验与非模态更新浮窗”条目。
   - 补充“独立关于对话框与版本/构建元信息”条目。
   - 在“请求”一节补充：可在设置的“编辑器”中开启“在 URL 中直接编辑 Query 参数”（默认使用 Query 表；开启后查询参数留在地址栏中编辑，不使用独立的 Query 表，保存时与表格模式互相兼容）。

### 6.5 发布与回归验证矩阵

在实施完成后，需在多平台上执行如下全面验证矩阵：

```
┌───────────────────────────┬───────────────┬─────────────────────────────────────────────────────────┐
│ 验证项目                  │ 执行命令      │ 预期合格标准                                            │
├───────────────────────────┼───────────────┼─────────────────────────────────────────────────────────┤
│ 代码规范与 Lint 静态分析   │ make lint     │ cargo fmt 检查通过，clippy 无任何 warning                │
│ Wasm 跨平台编译检查       │ make check-web│ 在 cyberapi-web 下 cargo check wasm32 编译 100% 通过    │
│ 单元测试覆盖率            │ cargo test    │ 所有 core、gui、store 单元测试与新增用例全部 Pass        │
│ macOS 桌面端本地运行      │ make dev      │ 标题栏更新 Popover 顺畅打开，关于对话框弹出无白屏       │
│ 默认表格模式回归验证      │ 手工验证      │ 默认 query_in_url=false，粘贴带参 URL 自动拆入 Query 表 │
│ URL 直编模式与兼容验证    │ 手工验证      │ 开启设置后参数留在 URL 栏、隐藏独立 Query 表，切回表格模式数据不丢│
└───────────────────────────┴───────────────┴─────────────────────────────────────────────────────────┘
```

---
*（全篇完。本技术设计文档已对 CyberAPI 的四项新需求提供完整的系统架构、UI 交互流程、状态机模型与存储兼容性定义，可直接作为后续生产级代码实施的权威蓝图。）*
