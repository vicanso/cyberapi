# CyberAPI UI 与用户体验（UX）巡检台账

本文件记录 CyberAPI 跨轮次 UI/UX 巡检的发现、修复闭环以及已确认的设计取舍，避免重复误报已知约定，并沉淀各轮真窗口验证结果。

巡检规范见 [ui-ux.md](file:///Users/tree/github/cyberapi-gpui/.agents/rules/ui-ux.md)、[ui-inspection.md](file:///Users/tree/github/cyberapi-gpui/.agents/rules/ui-inspection.md)，执行流程与隔离工具见 [cyberapi-ui-inspection](file:///Users/tree/github/cyberapi-gpui/.agents/skills/cyberapi-ui-inspection/SKILL.md)。

---

## 1. 已确认的设计取舍与已知限制（Accepted Trade-offs）

以下行为经确认属于有意设计或当前边界取舍，后续巡检除非产品规则变更，不重复列为缺陷：

| 编号 | 主题 | 当前行为与原因 | 参考位置 |
| --- | --- | --- | --- |
| AT-1 | 界面字号上限 `18px` | `TextPrefs::UI_SIZES` 限制在 `12..=18px`（默认 `14px`），因工作区部分行/头采用固定像素高度，更大字号会导致行高溢出。 | [prefs.rs](file:///Users/tree/github/cyberapi-gpui/crates/cyberapi-core/src/prefs.rs#L152-L160) |
| AT-2 | 设置窗口锁定打开时 `rem` | 设置界面与网页弹窗外包 `fixed_rem`，在打开期间调节界面字号时尺寸不变（下次打开生效），避免步进按钮从指针下移走。 | [fixed_rem.rs](file:///Users/tree/github/cyberapi-gpui/src/views/fixed_rem.rs) |
| AT-3 | 响应面板保留自定义查找栏 | 响应编辑器保持 `.searchable(false)` + 自定义搜索栏（配合 `shown_filter` / JSONPath 提示），不启用组件库内置悬浮查找框。 | [response_panel.rs](file:///Users/tree/github/cyberapi-gpui/src/views/workspace/response_panel.rs) |
| AT-4 | 弹窗内二次确认用内联确认 | 虽 `gpui-kit 0.11.0` 已支持叠层弹窗，为避免视觉遮罩过重与焦点链混乱，弹窗内危险操作仍优先使用 `inline_confirm`。 | [inline_confirm.rs](file:///Users/tree/github/cyberapi-gpui/src/views/inline_confirm.rs) |
| AT-5 | 网页端桌面专属能力隐藏/禁发 | 网页版因浏览器限制与安全边界，对 Mock、压测、独立窗口、客户端证书、本地文件/`env(...)`、WebSocket 与 gRPC 双向流等能力通过 `this_build::*` 诚实隐藏或提示不可用。 | [this_build.rs](file:///Users/tree/github/cyberapi-gpui/src/this_build.rs) |
| AT-6 | `views::fix_editor_search` 历史规避待清理 | `gpui-kit 0.11.0` 已在 `overlay.rs` 修复 `anchor_offset` 回显全选问题；旧调用点暂保留，待实机验证后统一清理。 | [views.rs](file:///Users/tree/github/cyberapi-gpui/src/views.rs#L123-L132) |

---

## 2. 巡检轮次汇总

| 轮次 | 日期 | Commit | 覆盖范围 | P0 / P1 / P2 | 结论 |
| --- | --- | --- | --- | --- | --- |
| Round 1 | 2026-11-25 | `e8ecba0` | 桌面端首页（空/有内容、浅/深）、工作区（空/多协议/Scratch、浅/深）、窄窗 `900×600` + `18px` + `en`/`zh-CN`、`⌘K` 命令面板、`⌘,` 设置窗 | 0 / 3 / 4（7/7 已验证） | 3 个 P1 溢出/重叠与 4 个 P2 体验项已全部修复并在隔离真窗口复验通过 |

---

## 3. 单轮巡检记录模板

> 新增巡检记录时，复制下方模板追加到本节下方（倒序排列，最新轮次在最前）。

```markdown
### Round N — YYYY-MM-DD（`<short-sha>`）

- **环境**：macOS `<version>` / Desktop Debug（`CYBERAPI_DATA_DIR=/tmp/...`）[/ Web Bridge `127.0.0.1:17380`]
- **覆盖矩阵**：
  - 首页（空态 / 有内容，浅色 / 深色）
  - 工作区（空态 / 选中态，HTTP / GraphQL / gRPC / WS / SSE，浅色 / 深色）
  - 语言与字号（`zh-CN` / `en`，`14px` / `18px`）
  - 窄窗（`900x600`）与弹窗 / 设置窗 / 独立窗口
  - 异常与护栏（离线、断言失败、搜索无匹配、`inline_confirm`、`UndoDelete`、`production_confirm`）
- **日志健康**：`stderr.log` / `cyberapi-dev.log.*` 0 panic / 0 异常错误

| ID | 级别 | 维度 | 现象与复现步骤 | 源码位置 | 截图 / 复验 | 状态 |
| --- | --- | --- | --- | --- | --- | --- |
| R1-01 | P1 | 视觉与布局 | 示例：描述具体现象与触发步骤 | `src/views/...:123` | `01-shot.png` -> `01-fixed.png` | 待修复 / 已验证 |
```

---

### Round 1 — 2026-11-25（`e8ecba0`）

- **环境**：macOS `27.2.1`（Host 浅深色 `Dark`）/ Desktop Debug（`CYBERAPI_DATA_DIR=/tmp/cyberapi-inspect-r1` 与 `/tmp/cyberapi-inspect-r1-empty`，隔离 `CYBERAPI_LEGACY_DIR`）+ 本地 Mock（`127.0.0.1:18080`）
- **覆盖矩阵**（共 19 张截图，保存于会话产物目录 `ui-inspect-r1/`）：
  - 首页空态：系统默认（跟随宿主机 Dark/en `01`）、强制 Light/zh-CN（`01b`）、Dark/zh-CN（`02`）
  - 首页有内容态（4 项目、折叠组、置顶、来源 sync 标）：`1280×800` 浅/深（`03`、`04`），`900×600` + `18px` + `en`（`16`）
  - 工作区空态：浅/深（`05`、`06`）
  - 工作区多协议选中态（`1280×800`）：HTTP（`07`、`08`）、GraphQL（`09`）、gRPC（`10`）、WebSocket（`11`）、SSE（`12`）、Scratch 临时请求（`13`）
  - 窄窗 + 最大字号：`900×600` + `18px` + `en` Light（`14`）、`900×600` + `18px` + `zh-CN` Dark（`15`）
  - 命令面板（`⌘K`）：`17-palette-light.png`、`18-settings-window-1-105549.png`（注：应用无 `⌘,` 快捷键，且 `18` 仅触发命令面板高亮 `Settings...` 未进入独立设置窗口）
- **日志健康**：`EMPTY_DIR` 与 `SEED_DIR` 的 `stderr.log` 均为空，`logs/cyberapi-dev.log.2026-11-25` 内 `0` panic、`0` `ERROR`、`0` `WARN`

| ID | 级别 | 维度 | 现象、修复方案与复现路径 | 源码位置 | 截图 / 复验 | 状态 |
| --- | --- | --- | --- | --- | --- | --- |
| R1-01 | P1 | 视觉与布局 / 交互阻断 | **现象**：标题栏有 7 个置顶接口时，在 `1280×800` 与 `900×600` 下仍将 `pins-all`（⌄）与设置按钮（⚙）挤出右侧。<br>**修复**：`flex_1()` 的 `0%` basis 在 Taffy `MinContent` 测算下视为不确定尺寸从而忽略 `w(px(0.))`；在标题栏容器、`render_title` 与 `PinsBar` 的 `flex_1()` 后显式补 `.flex_basis(px(0.))` 与 `.overflow_hidden()`。 | [root.rs](file:///Users/tree/github/cyberapi-gpui/src/root.rs#L351-L357), [root.rs](file:///Users/tree/github/cyberapi-gpui/src/root.rs#L696-L717), [pins.rs](file:///Users/tree/github/cyberapi-gpui/src/views/workspace/pins.rs#L178-L195) | `07` / `08` / `14` / `15`（原问题）-> `ui-inspect-r1-fixed/{03,07,08,14,15}-*.png`（⌄ 与 ⚙ 均完整可见） | 已验证 |
| R1-02 | P1 | 视觉与布局 | **现象**：`900×600` 窄窗 + `18px` 界面字号下，响应状态栏右侧徽章（`584 B` / `断言 2/3`）叠画在右侧按钮上。<br>**修复**：左侧状态容器显式 `.flex_1().w(px(0.)).min_w_0().overflow_hidden()`，延迟/徽章不折行，响应大小 `.min_w_0().truncate()` 并附 tooltip，右侧按钮组固定 `.flex_none()`。 | [response_panel.rs](file:///Users/tree/github/cyberapi-gpui/src/views/workspace/response_panel.rs#L724-L895), [response_panel.rs](file:///Users/tree/github/cyberapi-gpui/src/views/workspace/response_panel.rs#L2460-L2469) | `ui-inspect-r1/14,15` -> `ui-inspect-r1-fixed/14,15`（无重叠） | 已验证 |
| R1-03 | P1 | 视觉与布局 | **现象**：`900×600` + `18px` 下请求与响应 `TabBar` 后缀按钮遮挡标签且缺少溢出入口。<br>**修复**：为 `request-tabs` 与 `response-tabs` 启用 `.menu(true)` 溢出下拉，并为所有 `.suffix(...)` 添加 `.flex_none().pl_1()`。 | [request_editor.rs](file:///Users/tree/github/cyberapi-gpui/src/views/workspace/request_editor.rs#L2262-L2272), [response_panel.rs](file:///Users/tree/github/cyberapi-gpui/src/views/workspace/response_panel.rs#L934-L1089) | `ui-inspect-r1/14,15` -> `ui-inspect-r1-fixed/14,15`（显示 ▾ 溢出菜单且无重叠） | 已验证 |
| R1-04 | P2 | 视觉与布局 | **现象**：`KvEditor` 说明列在长文本下缺少右内边距与溢出裁剪，贴近 `×` 删除按钮半字裁切。<br>**修复**：为名称/值/说明单元格补充 `.overflow_hidden()`，为 `kv-desc-cell` 增加 `.pr_1()` 与 hover 完整内容 tooltip。 | [kv_editor.rs](file:///Users/tree/github/cyberapi-gpui/src/views/kv_editor.rs#L764-L870) | `ui-inspect-r1/07,08,14` -> `ui-inspect-r1-fixed/07,08,14` | 已验证 |
| R1-05 | P2 | 用户体验 | **现象**：空项目工作区下树形栏「全部折叠」按钮未禁用。<br>**修复**：根据 `rows()` 是否存在已展开目录计算 `has_open_folder`，为 `tree-collapse` 绑定 `.disabled(!has_open_folder)`。 | [tree_panel.rs](file:///Users/tree/github/cyberapi-gpui/src/views/workspace/tree_panel.rs#L460-L533) | `ui-inspect-r1/05` -> `ui-inspect-r1-fixed/05-workspace-empty-light.png` | 已验证 |
| R1-06 | P2 | 多协议与文案诚实度 | **现象**：WebSocket 下响应空态仍显示 HTTP「发送」提示。<br>**修复**：新增 `response.ws_empty_description` 双语文案，在 `HttpMethod::WebSocket` 下显示插头图标与「连接」操作指引。 | [response_panel.rs](file:///Users/tree/github/cyberapi-gpui/src/views/workspace/response_panel.rs#L2547-L2592), [zh-CN.toml](file:///Users/tree/github/cyberapi-gpui/locales/zh-CN.toml), [en.toml](file:///Users/tree/github/cyberapi-gpui/locales/en.toml) | `ui-inspect-r1/11` -> `ui-inspect-r1-fixed/11-workspace-ws-dark.png` | 已验证 |
| R1-07 | P2 | 平台规范与可达性 | **现象**：未绑定 `⌘,` 打开设置窗口；`⌘K` 命令面板底部长条目无分割线且贴边半行裁切。<br>**修复**：新增 `OpenSettings`、`cmd-,` 全局快捷键与 macOS「Settings…」菜单项；命令面板列表高度改整行 `px(384.)` 并为底栏加 `.pt_2().border_t_1()`。 | [lib.rs](file:///Users/tree/github/cyberapi-gpui/src/lib.rs#L71-L75), [lib.rs](file:///Users/tree/github/cyberapi-gpui/src/lib.rs#L454-L485), [palette.rs](file:///Users/tree/github/cyberapi-gpui/src/views/workspace/palette.rs#L447-L462) | `ui-inspect-r1/17` -> `ui-inspect-r1-fixed/17-palette-light.png`, `18-settings-cmd-comma-2-105869.png` | 已验证 |
