# 0.4.1 之前的排查：bug 与性能

2026-10-30。范围是 v0.4.0 之后的提交（B7、D6、G8、D5、F6、F5、A6、D7、B8、A7、F7，`9e9b112`…`cbf8a58`）里的 bug，加上整个应用的性能热点。只读代码，没有改动；分四路审查，每条结论回到代码核对过。“核实”一栏：**已核实**是对照代码确认了路径（个别连同依赖的源码）；**读代码确认**是审查时沿路径读过，没有再单独复核；**可能**是路径存在、触发条件没有确认。都还没有在运行的应用里复现。

编号：`BUG-n` 是错误行为，`PERF-n` 是性能，`Q-n` 是要先定做法的问题。修的时候一项一个提交，提交信息写编号（`fix(BUG-1): …`，用 `git log --grep BUG-1` 找到）；修完在汇总表的“核实”一栏和正文标题后标“已修复”。

## 建议的顺序

1. **先修**：BUG-1（可让 bridge 进程退出）、BUG-2、BUG-3、BUG-4、BUG-5（F7、F6、D6 的数据和发送问题）。
2. **接着**：BUG-6…BUG-12，和影响最大的性能问题 PERF-1…PERF-4。
3. **其余**按严重度排；Q 开头的先定做法。

2026-10-30 维护者的决定：压测（A7）相关的暂不优化——Q-5、BUG-15、PERF-12 留着不做；其余按这份文档的建议做：Q-1…Q-4 照各条的“建议”，BUG-22 导出时转回 3.0 的写法，PERF-6 流的记录上限 64 MB（到上限后不再记新的、保留开头，并写明截断；连上后计时每秒一次），PERF-15 加 bridge 选项 `--max-sends` / `CYBERAPI_BRIDGE_MAX_SENDS`（默认 64）。

## 汇总

| 编号 | 功能 | 严重度 | 一句话 | 核实 |
|---|---|---|---|---|
| BUG-1 | D7 | 高 | `now()` 的偏移大到溢出时 panic，release 构建 `panic = "abort"`，桌面版、命令行和经 MCP 发送的 bridge 整个进程退出 | 已核实；已修复 |
| BUG-2 | D6 | 中高 | 生产环境确认后，请求里有 `prompt()` 时询问弹窗刚打开就被关掉，请求永远发不出去 | 已核实；已修复 |
| BUG-3 | F6 | 中高 | 删除“仅本地”的条目再撤销，它回来时不再是仅本地，下次同步会推给团队 | 读代码确认；已修复 |
| BUG-4 | F7 | 中 | 独立窗口里请求编辑器的标签页跟着主窗口当前的接口走，点它会改别的接口的标签页 | 已核实；已修复 |
| BUG-5 | F7 | 中 | 标签被重建（同事的修改到达、放弃本地修改）后关闭独立窗口，旧请求被写回存储，下次同步盖掉同事的修改 | 已核实；已修复 |
| BUG-6 | F6 | 中 | 有一个备份的文件名时间晚于现在（时钟调回、跨时区），自动备份变成每小时一次，一周的备份被冲掉 | 已核实；已修复 |
| BUG-7 | A7 | 中 | 压测结束时的用时和吞吐包含最多约 0.6 秒的刷新等待和准备时间，短测试的吞吐被低估很多 | 已核实；已修复 |
| BUG-8 | A7 | 中 | gRPC 调用失败（HTTP 200 + 非 0 的 `grpc-status`）算作成功 | 读代码确认；已修复 |
| BUG-9 | F6 | 中低 | 撤销计数会重用、旧通知的“撤销”按钮点了没反应；第一次删除的计时器会提前结束第二次的撤销 | 已核实；已修复 |
| BUG-10 | F6 | 中低 | 撤销先取走记录再执行；和正在保存的接口撞上得到 `Stale` 时只报错，删除变成永久 | 可能；已修复 |
| BUG-11 | A6 | 中低 | 导入的 Schema 行改成别的检查或停用后仍标着“来自导入”，从来源同步时被删掉换成新的 | 读代码确认；已修复 |
| BUG-12 | A6 | 中低 | OpenAPI 3.0 转 JSON Schema：`$ref` / `allOf` 旁的 `nullable` 丢失；`properties` 下名叫 `nullable`、`example` 等的字段被当成关键字处理 | 读代码确认；已修复（体积问题未处理） |
| BUG-13 | B7 | 中低 | “从来源同步”弹窗里改了地址但没重新获取，旧地址取回的结果显示在新地址下，同步合并的是旧内容 | 读代码确认；已修复 |
| BUG-14 | B7 | 低 | 导入弹窗获取过一个地址后，把内容整个换掉再导入，项目仍记住那个地址作为来源 | 读代码确认 |
| BUG-15 | A7 | 中 | 每次发送都新建连接：时间里含握手；高速长时测试会用光本机端口，本机错误被算成服务器失败 | 可能；暂不处理 |
| BUG-16 | B8 | 低中 | 请求行前有 JetBrains 的预处理脚本（`< {% %}`）时这条请求被静默丢掉；文件以它开头时整个文件认不出 | 读代码确认 |
| BUG-17 | F7 | 低 | 独立窗口里弹窗关闭后焦点丢失：⌘W 隐藏整个应用，⌘↵ 不发送 | 可能 |
| BUG-18 | F7 | 低 | 独立窗口总是开在主显示器上 | 读代码确认 |
| BUG-19 | 设置 | 低 | 几处项目设置的写入都从旧副本出发，两次快速修改时前一次丢失（如标生产环境后马上点变量的锁） | 可能 |
| BUG-20 | G8 | 低中 | 版本对比里请求体、说明、示例各是一整行，改了什么看不出来；gRPC 的描述符可能是几百 KB 的一行 | 读代码确认 |
| BUG-21 | G8 | 低 | 对比按版本在列表里的位置记，恢复或刷新后列表变了，对比的就不是原来那个版本 | 可能 |
| BUG-22 | A6 | 低 | 导出 `openapi: 3.0.3` 时写入 2020-12 的 Schema（`type: [x, "null"]`、数值形式的 `exclusiveMinimum`、`$defs`），别的工具会拒绝 | 读代码确认 |
| BUG-23 | F5 | 低 | 桌面版复制进服务器副本时用内存里缓存的绑定，基准修订号可能是旧的，下次打开出现不该有的冲突 | 可能 |
| BUG-24 | F6 | 低 | 撤销通知持有工作区的强引用：离开项目再回来，点旧通知会在已分离的状态上撤销（网页会用旧草稿盖掉新的） | 读代码确认 |
| BUG-25 | B8 | 低 | multipart 某段内容以 `<` 开头（XML、HTML）被当成读文件 | 读代码确认 |
| BUG-26 | B8 | 低 | 带 UTF-8 BOM 的 `.http` 文件认不出 | 可能 |
| BUG-27 | D7 | 低 | `md5` / `sha*` / `hmac*` / `urlEncode` 都按文本取输入，链在 `base64Decode` 或 `readFile` 后面时非 UTF-8 字节被换成 U+FFFD，结果是错的 | 读代码确认 |
| PERF-1 | 树 | 高 | 树面板每帧重建所有行（含关键字搜索、拼音匹配），不用 `uniform_list` | 已核实；已修复 |
| PERF-2 | GraphQL | 高 | GraphQL 响应在每帧和每次编辑时整份解析 `errors` | 已核实；已修复 |
| PERF-3 | 响应 | 高 | 大响应先整份格式化再截断，都在 UI 线程上，每次切回标签再做一遍 | 已核实；部分修复 |
| PERF-4 | bridge | 高 | 首页每 10 秒统计全库所有项目的接口数再过滤；页面同时重读全部草稿 | 已核实；已修复 |
| PERF-5 | 运行器 | 中高 | 运行弹窗每帧克隆并渲染全部结果 | 读代码确认 |
| PERF-6 | 流 | 中高 | 打开着的流每 100ms 重绘整个窗口直到关闭；WebSocket / gRPC 流的收发记录没有上限且存三份 | 读代码确认 |
| PERF-7 | bridge | 中高 | 同步 / 上传逐行写入，约 3 ×（接口 + 目录）次往返，全程持锁 | 读代码确认 |
| PERF-8 | 内存 | 中 | 每个标签把新收到的完整响应体（最多 64 MB）留在内存里，最多 history_size 条 | 已核实 |
| PERF-9 | 历史 | 中 | 打开标签时读全部历史连同响应体；桌面版占着唯一的数据库连接 | 读代码确认 |
| PERF-10 | 编辑器 | 中 | 请求编辑器每帧克隆整个请求三次；每次按键整份解析请求体 | 读代码确认 |
| PERF-11 | 命令面板 | 中 | 打开命令面板读最多 50 个项目的全部内容，每帧对全部接口做拼音匹配 | 读代码确认 |
| PERF-12 | A7 | 中 | 压测每 300ms 在 UI 线程上排序全部样本（最多 100 万） | 已核实；暂不处理 |
| PERF-13 | 网页 | 中 | 服务器项目每次修改都把整个项目序列化进 localStorage | 读代码确认 |
| PERF-14 | bridge | 中 | PostgreSQL 的 `responses` 没有以 `api` 开头的索引，删接口时级联删除逐条全表扫描 | 已核实 |
| PERF-15 | bridge | 低中 | `/v1/send` 只按账号限并发，没有全局内存上限 | 读代码确认 |
| PERF-16 | G8 | 低 | 版本对比展开接口时 `diff::compare` 在 UI 线程上同步执行（响应对比是放到后台的） | 读代码确认 |
| Q-1 | B7 | 低 | 来源地址是项目数据，谁都能改；别人打开“从来源同步”时桌面版立即去取，可指向本机或内网 | 读代码确认 |
| Q-2 | B7 | 低 | 预览里“来源里已经没有”的列表覆盖整个项目，包括手写的接口 | 读代码确认 |
| Q-3 | B7 | 低 | 从顶层重新导入来源地址时，来源记的目录被悄悄改成顶层 | 读代码确认 |
| Q-4 | D5 | 低 | 目录 / 接口变量的输入框有模板函数提示，但这些值按原样使用，不求值 | 读代码确认 |
| Q-5 | A7 | 低中 | 压测发的是保存了的请求，不是编辑器里的：查看者的本地修改、还在防抖中的修改都不在其中 | 读代码确认；暂不处理 |

## Bug

### BUG-1 `now()` 的偏移溢出时 panic，进程退出（D7，高）——已修复

- 位置：`crates/cyberapi-core/src/template.rs:581`（`DateTime + TimeDelta`），`shift` 在 `:623`。
- 场景：请求里写 `{{now(%Y, 100000000d)}}`。`TimeDelta::try_days(1e8)` 在范围内返回 `Some`，相加时超出 chrono 能表示的年份（262143）。chrono 0.4.45 的 `Add` 是 `checked_add_signed(rhs).expect("`DateTime + TimeDelta` overflowed")`，于是 panic；负的同样。
- 影响：根 `Cargo.toml` 的 `[profile.release]` 设了 `panic = "abort"`。桌面版在发送、复制为代码、运行时退出；命令行退出；**bridge 经 MCP `send_api` 在服务器上准备请求，任何编辑者存一个这样的请求，再用能发送的令牌调用，就能让共用的 bridge 退出**。页面里是那个标签页崩溃。
- 修复方向：改用 `checked_add_signed`，溢出时返回模板错误；`shift` 也可以限一个合理范围（如 ±100 年）。加一条测试。
- 修复：偏移本身超出 `TimeDelta`（如 `9223372036854775807s`，以前被当成格式原样输出）和加到当前时间后超出日历，都返回参数错误（“a shift the calendar reaches”），请求不发出。没有另限范围：一个世纪以内的偏移照常可用。测试 `now_is_iso_8601_or_formatted_local_or_utc` 覆盖正负两个方向，去掉修复时它因 panic 失败。同一文件里其他函数看过：`random` 用 i128 算区间，`randomString` 有长度上限，不会因输入 panic。

### BUG-2 生产环境确认后，有 `prompt()` 的请求发不出去（D6，中高）——已修复

- 位置：`src/views/workspace/production_confirm.rs:34`，`request_editor.rs:1190-1215`，`views/name_dialog.rs:36-37`。
- 场景：当前环境标为生产，POST 请求里有 `{{prompt(...)}}`，点发送，再在确认框点“发送”。
- 原因：确认框的 `go` 调用 `send_unasked`，后者同步打开询问弹窗（`open_prompt_dialog` → `open_dialog`），`go` 再返回 `true`，`ok_button` 随即 `close_dialog`。gpui-component 0.8.1 的 `close_dialog_internal` 关的是**最新的**弹窗（`rposition(|d| !d.closing)`），于是刚打开的询问弹窗被关，确认框留着。每次都这样。网页版还违反了“弹窗上不叠弹窗”。
- 绕过：勾“到退出前不再询问”，取消，再从编辑器发送。
- 修复方向：`go` 先关掉确认框（返回 `false` 并自己 `close_dialog`），再 `cx.defer` 调用 `send_unasked`；或者把询问并进确认框里（和运行弹窗一样就地询问）。
- 修复：`confirm_production` 的“发送”先 `close_dialog` 关掉确认框（此时它是最新的弹窗），再调用 `send`，返回 `false` 让 `ok_button` 不再关一次。不需要 `defer`：组件库的关闭只是把确认框标为 closing（动画结束后移除），随后打开的询问弹窗是新的一层，不会被关。所有用它的路径（发送、WebSocket / gRPC 流的连接）都受益。测试 `the_dialog_a_send_opens_stays` 在真实的弹窗层上点“发送”，再点 send 自己打开的弹窗里的按钮；改回原来的顺序时它失败。

### BUG-3 撤销删除的“仅本地”条目后，它会被同步出去（F6，中高）——已修复

- 位置：`src/states/workspace.rs:2764-2803`（`undo_delete`），`src/states/copy.rs:121-136`（`recount`），`crates/cyberapi-core/src/local_only.rs:202-204`（`keep`）。
- 场景：在服务器项目（网页或桌面副本）里删掉一个标为仅本地的条目，点“撤销”。
- 原因：删除经 `CopyStore::write` → `recount` → `local.keep(&config)`，`keep` 里 `roots.retain(|root| present.contains(root))` 去掉了这个标记并存下。撤销用 `put_back` + `replace` 按原 id 放回，但没有重新标为仅本地；下一次 `snapshot` → `syncable` 就把它连同子树推给团队。
- 修复方向：`Undo` 记下删除时去掉的仅本地根，撤销成功后 `ServerCopy::set_local` 放回；加一条副本的状态测试。
- 修复：`Undo.local` 记下删除前被删子树里的仅本地根；撤销的那个存储调用在 `replace` 之后接着执行 `ServerCopy::put_back_local`，它在运行时（而不是创建时）重新标记，再和 `set_local` 一样保存标记、按可同步部分重算是否未同步（共用的尾部抽成 `copy.rs` 的 `local_settled`）。放在同一个调用里，是因为撤销前先保存打开的接口，那些写入的 `recount` 会把提前加上的标记再去掉。测试 `the_readers_own_items_stay_here` 加了删除再撤销：条目回来、仍是仅本地、没有要同步的；去掉重新标记时它失败。

### BUG-4 独立窗口的请求标签页跟着主窗口当前的接口走（F7，中）——已修复

- 位置：`src/states/workspace.rs:1389-1405`（`tab()` / `set_tab()` 都读 `self.api`，即主窗口当前打开的接口），`request_editor.rs:2091`、`:1626`，`response_panel.rs:712`（断言计数的点击）、`:1040`（打开接口设置）。
- 场景：A 放进独立窗口，主窗口点开 B（独立窗口本来就是为了这样对照）。A 的窗口显示的是 B 选中的标签页；在 A 的窗口点 Body、Headers，改的是 B 记下的标签页，主窗口的编辑器也跟着切。主窗口没有打开的接口时 `set_tab` 直接返回，A 的窗口停在 Query。
- 另外：A 的窗口没有订阅 `EditorShare`，编辑器宽度预设的下拉不起作用。
- 修复方向：`tab` / `set_tab` 改成按接口 id（编辑器把自己的 `ApiState` 的 id 传进去）；独立窗口处理 `EditorShare`（或隐藏这个下拉）。
- 修复：`WorkspaceState::tab` / `set_tab` 收调用方的 `Entity<ApiState>`，请求编辑器的标签栏、响应区的断言计数和“打开接口设置”都传自己的。独立窗口订阅编辑器的 `EditorShare`，按比例调整自己的两栏（只对这个窗口，不存进设置；下拉里打勾的仍是主窗口的比例）。测试 `the_tab_is_kept_per_api` 加了：打开 `b` 时给 `a` 选标签页，`a` 是新选的、`b` 不变；让 `tab` 改回读当前打开的接口时它失败。

### BUG-5 标签重建后关闭独立窗口，旧请求被写回（F7，中）——已修复

- 位置：`src/views/workspace/apart_window.rs:123-133`（`on_release` 里无条件 `editor.flush`），`states/workspace.rs` 的 `start_again`、`took`，`states/api.rs` 的 `set_uri` → `save_now`。
- 场景：服务器项目里，在独立窗口的地址栏改了地址，没按回车也没离开输入框就切到别的窗口（窗口间切换不会让输入框失焦，`ApiState` 里还没有这次修改）。随后同事改了这个接口，轮询拉到后 `took` → `start_again`：从 `kept` 和 `apart` 里拿掉旧的 `ApiState` 并 `forget_unsaved`。独立窗口随即关闭，`on_release` 把编辑器里的内容 `flush` 进旧的 `ApiState`，地址不同于是 `save_now`，旧请求加新地址存进副本，副本标为未同步；下次同步就把同事的修改盖掉。“放弃本地修改”（`take_server`）和导入时“更新已有的”同样会被这样撤回。
- 对照：主窗口在 `start_again` 时直接丢弃编辑器，不 `flush`，这是设计。
- 修复方向：`on_release` 里只有 `kept_api(id)` 仍是这个窗口的 `ApiState` 时才 `flush`；加一条测试。
- 修复：照上面做。读者自己关掉窗口（⌘W、关闭按钮）、离开项目时标签还在，照旧写回；标签被重建或在主窗口里被关掉时，窗口里打了但还没回车或离开输入框的内容不再写回（和关掉临时请求一样丢掉）。测试 `closing_writes_back_only_the_tab_still_open` 在窗口的地址栏里打字（不交给 `ApiState`）：直接关窗口时地址存下；再打一次、随后“更新已有的”导入重建了这个标签，存储里仍是导入的请求（地址和新请求头）。无条件 `flush` 时它失败，存下的是旧地址。触发条件也由这条测试确认了。

### BUG-6 一个“未来”的备份让自动备份每小时一次（F6，中）——已修复

- 位置：`crates/cyberapi-core/src/backup.rs:73-90`（`due`、`pruned`）；文件名是本地时间、不带时区（`app.rs` 的 `back_up_by_itself`）。
- 原因：`due` 返回 `newest > now || now - newest >= EVERY`。新备份按当下时间命名，比那个“未来”的文件早，最新的仍是它，于是每次检查（每小时）都到期。
- 场景：笔记本从东八区到东一区，或时钟曾被调快过。每小时写一份完整快照；`pruned` 按名字里的时间留最新 7 份，约 6 小时后一周的每日备份全被小时备份替换。若现有 7 份都晚于现在，新写的那份会立刻被删，每小时重复。
- 修复方向：文件名用 UTC（或带偏移）；最新的在未来时按“今天还没备份过”判断（比如看文件的修改时间），不要每次都到期。补测试。
- 修复：文件名不变（手动备份和旧版都用这个格式）。`due` 和 `pruned` 按名字超前现在多少区分：26 小时以内（往西跨时区）算刚做过的，不到期；超过 26 小时（做的时候时钟错了）的不计入是否到期，清理时当作最旧的、先删。于是时钟调回或往西出差时仍是一天一份，新做的不会被立即删掉。`pruned` 多了 `now` 参数。测试 `a_backup_ahead_of_the_clock_keeps_the_day`；改回“在未来就到期”时它失败。

### BUG-7 压测的用时和吞吐包含等待（A7，中）——已修复

- 位置：`src/states/load.rs:139-160`。
- 原因：`frames` 每批之后睡 `FRAME`（300ms），发送结束、通道关闭前后最多还要再睡两次，之后才用 `started.elapsed()` 算用时和吞吐；`started` 又是在准备请求（取 Cookie、申请 OAuth 令牌）之前记的。
- 例子：默认的 10 并发 × 100 次打一个 20ms 的接口，实际约 0.2 秒，弹窗显示约 0.62 秒、约 160 次/秒（应约 500）。
- 修复方向：用样本里最晚的 `Sample.at`（已经是从测试开始量的）作为用时；运行中显示的用时同理。
- 修复：`LoadState` 记下目前最晚的 `Sample.at`（`sent_for`，每批增量更新，不扫描全部样本）。跑完时用它作用时和吞吐的分母；运行中的吞吐也用它，界面上走动的计时仍按墙上时间。手动停止时仍用从开始到停止的墙上时间：卡住的服务器不该让吞吐看起来更高。测试 `a_test_takes_as_long_as_its_sends`；改回 `started.elapsed()` 时它失败。

### BUG-8 压测把失败的 gRPC 调用算作成功（A7，中）——已修复

- 位置：`crates/cyberapi-service/src/load.rs:58-66`，`crates/cyberapi-core/src/load.rs:85-87`。
- 原因：只有 `status >= 400` 算失败；gRPC 总是 HTTP 200，错误在 `grpc-status`（trailers，或只有 trailers 的应答在头里）。一个总是返回 UNAVAILABLE 的一元调用显示错误率 0%。
- 修复方向：对 gRPC 读 `GrpcStatus`，非 0 算失败，结果里按 gRPC 状态分类。
- 修复：`Sample.grpc` 记下 gRPC 调用的状态码（trailers，或只有 trailers 的应答的头；都没有算 UNKNOWN），非 0 算失败；HTTP 本身出错（如代理的 502）仍按 HTTP 状态。结果多一种 `Outcome::Grpc`，弹窗里按状态名显示（`UNAVAILABLE`）。测试：core 的 `a_grpc_call_fails_by_its_status`，以及服务的 gRPC 测试里加的一次压测（HTTP 200 + `grpc-status: 14` 算失败）；`failed()` 去掉 gRPC 的判断时两者都失败。

### BUG-9 撤销的计数会重用，旧通知的按钮无效（F6，中低）——已修复

- 位置：`src/states/workspace.rs:2657`（`count = this.undo.map_or(0, |undo| undo.count) + 1`），`src/views/workspace.rs:564-593`。
- 场景一：删 A，再删 B。A 的通知还在，点它的“撤销”：`undo_delete(1)` 因为 `count != 2` 直接返回，通知却被移除，读者以为 A 回来了。
- 场景二：删 A（计数 1）、撤销，再删 B（计数又是 1）。A 那条 10 秒的计时器到点调用 `forget_undo(1)` 并移除 1 号通知，B 的撤销被提前结束。
- 修复方向：计数用单调递增的字段，不从当前记录推算；旧通知在新删除时移除，或点击无效时提示“只能撤销最近一次”。
- 修复：`WorkspaceState.deletes` 每次告知读者的删除加一，作为这次的编号；工作区视图弹出新的撤销通知前移除上一条（编号减一），屏幕上只剩能撤销的那条。测试 `a_delete_is_undone` 加了：撤销后再删的编号是 2 而不是又一个 1；第三次删除后，前两次的计时器到点（`forget_undo(1)` / `forget_undo(2)`）不影响撤销第三次。改回从当前记录推算编号时它失败。

### BUG-10 撤销失败时删除变成永久（F6，中低，可能）——已修复

- 位置：`src/states/workspace.rs:2768-2785`。
- 原因：先 `self.undo.take()`，再对每个打开的接口 `save_now()`（走各自接口的 `run_serial`，不是工作区的）；撤销本身是 `config` → `replace(.., Some(revision))`。若某个保存落在这两步之间，`replace` 返回 `Stale`，只发一个 `Failed`；通知已被点击移除，记录也没了。
- 触发：在打开的接口里打字后 300ms 内点撤销。
- 修复方向：失败时把记录放回（并重新显示通知），或遇到 `Stale` 时重读再试一次。
- 修复：两样都做了。撤销写入得到 `Stale` 时重读一次项目再写；仍失败（或别的错误）时，只要这之后没有新的删除，就把记录放回并再发一次 `Deleted`，通知重新出现，可以再点。测试 `an_undo_a_save_beats_is_tried_again` 用一个在每次整份读取之后插入别人写入的树存储：插入一次时撤销成功；插入两次时撤销失败、通知回来，再点一次成功。去掉重试时它失败。

### BUG-11 导入的 Schema 行改过之后仍算“来自导入”（A6，中低）——已修复

- 位置：`src/views/workspace/assertions_editor.rs:547-574`（`set_subject` 保留 `row.imported`，只有改 Schema 文本才清掉），`crates/cyberapi-core/src/import/merge.rs:75-104`（`updated`）。
- 场景：把导入的那条 Schema 断言改成“状态码等于 200”，它仍是 `imported: true`（界面上已不显示“来自导入”）；下次从来源同步时被删掉，换上一条新的 Schema 断言。把它停用同理，同步后换成启用的。
- 修复方向：改对象、改比较方式、停用 / 启用都清掉 `imported`；`updated` 只换掉对象仍是 Schema 的导入行。
- 修复：照上面做。断言编辑器改对象、改比较方式、点开关都清掉这一行的 `imported`，之后它是读者自己的（停用的 Schema 断言也算读者自己的，导入不会再加一条启用的）；`merge::updated` 只把对象仍是 Schema 的导入行当作导入的，所以修复之前已经存下的、改成别的检查的行也会留下。测试 `the_imports_assertions_are_updated` 加了这种行；`updated` 改回按标记过滤时它失败。编辑器那边没有视图测试。

### BUG-12 OpenAPI 3.0 转 JSON Schema 不完整（A6，中低）——已修复（体积问题未处理）

- 位置：`crates/cyberapi-core/src/import/openapi.rs:57-112`（`standalone`）。
- `nullable` 只在 `type` 是字符串时生效：3.0 常见的 `{"nullable": true, "allOf": [{"$ref": …}]}` 丢了 `nullable`；`{"$ref": …, "nullable": true}` 被整个换成目标。响应里出现 `null` 时断言失败。
- 关键字处理也作用在 `properties` 这类“名字 → Schema”的表上：名叫 `nullable` / `x-nullable` 的属性被删掉（配合 `additionalProperties: false` 时合法的响应失败）；名叫 `default`、`example`、`examples`、`enum`、`const` 的属性被原样拷贝，里面的 `$ref` 不展开，编译失败，断言一直是“Schema 读不出来”。
- 附带：每个接口存一份展开后的 Schema（最多 4000 个节点），大的规范会让项目（网页草稿在 localStorage）大好几 MB。
- 修复方向：`properties`、`patternProperties`、`$defs` / `definitions` 按“名字 → Schema”处理；`nullable` 对没有 `type` 的情况写成 `anyOf: [原 Schema, {type: null}]`。补测试。
- 修复：`standalone` 先读 `nullable` / `x-nullable`，展开（含 `$ref`）之后由 `or_null` 允许 `null`：有 `type` 时加进类型（字符串或数组），有 `enum` 时也把 `null` 加进去，没有 `type` 时写成 `anyOf`。`properties`、`patternProperties`、`definitions`、`$defs`、`dependentSchemas` 按“名字 → Schema”逐个转换，属性叫什么都行。测试 `a_responses_schema_becomes_an_assertion` 加了 `allOf` / `$ref` 旁的 `nullable`、带 `enum` 的可空引用、叫 `nullable` 和 `example` 的属性；让没有 `type` 的可空 Schema 保持原样时它失败。“附带”那条（每个接口存一份展开的 Schema）没有处理：要改成项目里共用一份定义、断言引用它，是数据模型的改动，先记在这里。

### BUG-13 “从来源同步”合并了旧地址的内容（B7，中低）——已修复

- 位置：`src/views/workspace/source_dialog.rs:83-90`、`:106`、`:164`、`:175`。
- 场景：弹窗打开时开始取已记住的来源；读者把地址改成新的，没按回车也没点“获取”；旧的那次取完，预览出现在新地址下。点“同步”调用 `sync_from_source(旧地址, 旧内容)`，合并旧内容并把旧地址留作来源。
- 原因：输入框的 `Change` 只在已有 `fetched` 时清掉它，不取消进行中的获取；获取完成时直接 `this.fetched = Some(...)`，不核对当前输入的地址。
- 修复方向：获取结果带上它的地址，和当前输入不一致时丢弃；改动地址时取消进行中的获取。
- 修复：照上面做。`SourceView.fetching` 记下正在取的地址，输入框变成别的地址时丢掉这个任务（随之取消）；结果回来时再核对一次当前输入，不一致就不显示。弹窗没有视图测试。

### BUG-14 导入弹窗记住的来源地址和导入的内容不一致（B7，低）

- 位置：`src/views/workspace/import_dialog.rs:281`、`:344-368`、`:417`。
- 场景：获取了地址 A，再全选粘贴另一份集合（或手改），然后导入；“记住来源”默认勾着，项目记下 A。之后“从来源同步”把 A 的内容合并进来。
- 原因：只有 `fill()` 重置 `fetched_from`；小段粘贴走编辑器自己的粘贴，打字也不重置。
- 修复方向：编辑框内容变化且不再是获取到的原文时清掉 `fetched_from`。

### BUG-15 压测每次新建连接（A7，中，可能）——暂不处理（维护者：压测暂不优化）

- 位置：`crates/cyberapi-service/src/load.rs:58` → `NativeEngine::send`（`crates/cyberapi-http/src/native.rs:161`）。
- 影响：每个样本都含 TCP（HTTPS 还有 TLS 握手）的时间，和别的压测工具测的不是一回事；本机主动关连接留下 TIME_WAIT，macOS 约 16384 个临时端口、TIME_WAIT 约 30 秒，持续每秒 550 次以上超过半分钟就开始 “Can't assign requested address”。这些本机错误被算作服务器失败，且立即返回，按时长的测试很快到 `MAX_SENDS`，又加重 PERF-12。
- 修复方向：压测的引擎复用连接（keep-alive 连接池，每个并发一条）；至少把连接类错误单独归类、在结果里说明。界面说明里写清楚“含握手”。

### BUG-16 带预处理脚本的 JetBrains 请求被丢掉（B8，低中）

- 位置：`crates/cyberapi-core/src/import/http_file.rs:314-358`、`:137-143`。
- 原因：请求行前的 `< {% … %}` 或 `< ./pre.js` 走到 `break`，`request_line(...)` 返回 `None`，这条请求无声消失；文件第一条请求就带这样的脚本时，`is_http` 判断失败，落到 JSON / YAML 解析报错。
- 修复方向：请求行前的 `< {% … %}`（可跨行到 `%}`）和 `< 文件` 跳过；导入结果里说明丢了哪些脚本。

### BUG-17 独立窗口里弹窗关闭后快捷键失效（F7，低，可能）

- 位置：`src/views/workspace/apart_window.rs:176-200`；`src/lib.rs:443`（`CloseWindow` → `cx.hide()`）。
- 原因：主窗口的工作区在没有焦点时会把焦点拿回来，独立窗口不会。询问弹窗、生产环境确认关闭后没有焦点，键上下文没了：⌘W 落到全局的 `CloseWindow`，在 macOS 上隐藏整个应用；⌘↵ 没有处理者。
- 修复方向：独立窗口 `render` 时没有焦点就聚焦自己的 `FocusHandle`（照主窗口的做法）。

### BUG-18 独立窗口总是开在主显示器上（F7，低）

- 位置：`apart_window.rs:49-52`。
- 原因：`cx.active_window().update(...)` 在主窗口自己的更新里执行（`Parted` 的处理、“切换到那个窗口”按钮都是），对正在更新的窗口 `update_window_erased` 返回 `None`，`display` 总是 `None`，`Bounds::centered` 用主显示器。
- 修复方向：由主窗口在自己的回调里把 `window.display(cx)` 传给 `open_apart`。

### BUG-19 项目设置的连续修改丢掉前一次（低，可能）

- 位置：`set_production`、`set_secret`、`import_settings`、`forget_source`，都调用 `set_project_settings`（`src/states/workspace.rs`）。
- 原因：调用方各自从 `self.data.settings` 克隆出新设置，而 `set_project_settings` 只在 `run_serial` 的回调里才 `this.data.settings = settings`。两次修改都从旧值出发，后一次写入时丢了前一次（如标生产环境后马上点某个变量的锁）。
- 修复方向：写之前先把新值放进 `self.data.settings`（失败时回滚），或改成在串行队列里“读当前值 → 改 → 写”。

### BUG-20 版本对比里请求的修改看不出来（G8，低中）

- 位置：`crates/cyberapi-core/src/versions.rs:75-79`（`request_text`），`src/views/workspace/versions_dialog.rs:136-150`。
- 原因：`HttpRequest.body` 是字符串，缩进的 JSON 里它是一整行转义后的字符串；改请求体只显示一行删除一行新增，而且不换行、超出部分截掉。说明、示例同理；gRPC 的 base64 描述符可能是几百 KB 的一行，还要排版。
- 修复方向：`request_text` 把请求体（JSON 时格式化）、说明、示例各自展开成多行，描述符只写摘要（大小和哈希）。

### BUG-21 版本对比按列表位置记版本（G8，低，可能）

- 位置：`src/views/workspace/versions_dialog.rs`（`Comparing.at`）。
- 场景：恢复后 `load_versions` 在最前面插入新版本；正开着的对比，标题显示的是另一个版本的时间，切换“和现在 / 和下一版”时对比的也换了版本。
- 修复方向：按版本 id 记。

### BUG-22 导出的 OpenAPI 3.0 里写了 2020-12 的 Schema（A6，低）

- 位置：`crates/cyberapi-core/src/export/openapi.rs:257-283`。
- 原因：导入时已转成 2020-12（`type: [x, "null"]`、数值的 `exclusiveMinimum`），读者自己写的还可能有 `$schema`、`$defs`、`const`，这些在 3.0 里都无效，别的工具会拒绝。
- 修复方向：导出时转回 3.0 的写法（或导出为 `openapi: 3.1.0`，3.1 直接用 2020-12）。

### BUG-23 桌面版复制进服务器副本时基准修订号可能是旧的（F5，低，可能）

- 位置：`src/states/workspace.rs:2717-2734`。
- 原因：绑定取自 `ServerState::mine()` 的内存缓存，只有首页和复制会 `note_bound`；在工作区里同步（`ServerCopy::pushed` → `persist` → `bind`）不更新这份缓存。同步 B 后经命令面板到 A、再复制进 B，写下 `{base: 旧, unsynced: true}`，下次打开 B 出现不该有的冲突，选“用服务器的”就丢了复制进去的条目。
- 另：导入成功但 `bind` 或读变量失败时，复制已完成但报错，绑定可能没标未同步。
- 修复方向：`bind` 前从 `ServerCopyStore` 读当前绑定，而不是用缓存；或者同步时也 `note_bound`。

### BUG-24 撤销通知持有工作区的强引用（F6，低）

- 位置：`src/views/workspace.rs:571`。
- 场景：删除后离开项目、再打开并编辑，10 秒内点旧通知：撤销在已经分离的旧状态上执行，网页版会把旧的 `MemoryStore` 副本写成草稿，盖掉新会话的草稿。条目已经回来时（“用服务器的”、“放弃本地修改”之后）撤销仍会 `replace`，副本被标未同步、推一个没人改过的修订。
- 修复方向：通知持有 `WeakEntity`，离开项目时移除通知；`put_back` 没有改动时不写。

### BUG-25 multipart 的 XML 段被当成文件（B8，低）

- 位置：`http_file.rs:254-257`（`content.trim().strip_prefix('<')`）。
- 场景：`<?xml …?><a/>` 被导入成 `file://?xml …`。读文件的写法是 `< 路径`（带空格）。
- 修复方向：要求 `< ` 后跟空白，或只认单独一行的 `< 路径`。

### BUG-26 带 BOM 的 `.http` 文件认不出（B8，低，可能）

- 位置：`http_file.rs:132-145`，`import/mod.rs:200`。
- 原因：`trim` 不去 U+FEFF，第一行的 `###`、注释、`@变量`、方法都匹配不上，落到 JSON / YAML 解析报错。
- 修复方向：`detect` / `parse` 先去掉开头的 BOM（对所有文本格式都有好处）。

### BUG-27 摘要函数按文本取字节输入（D7，低）

- 位置：`template.rs:421-430`。
- 原因：`md5`、`sha*`、`hmac*`、`urlEncode` 都用 `into_text()`（`from_utf8_lossy`）。`{{sha256.base64Decode(AAEC/w==)}}` 算的是把 0xFF 换成 U+FFFD 后的文本，不是字节 `[0, 1, 2, 255]`。`md5.readFile` 原来就有这个问题，D7 多了一条路。
- 修复方向：这些函数对 `Value::Bytes` 直接用字节（新增 `into_bytes` 已有）。

## 性能

渲染开销为什么要紧：应用没有对视图用 `.cached()`，每一帧都会重新执行各视图的 `render`。帧不只来自输入：输入框光标每 500ms 闪一次（有焦点时每秒两帧），悬停、滚动会重绘（60–120 帧/秒），发送中的计时每 100ms 一次（PERF-6）。所以 `render` 里的活，在光标闪烁时每秒做两次，滚动时每秒做几十次。网页版的 `background_spawn` 也跑在页面唯一的线程上。

### PERF-1 树面板每帧重建所有行（高）——已修复

- 位置：`src/views/workspace/tree_panel.rs:789`（`state.rows()`）、`:862-878`（全部行生成元素放进普通 `div`，没有 `uniform_list`）、`:537`（每个目录行 `api_count`）、`:599`（`marked`）；`src/states/workspace.rs:1792-1795`。
- 情况：`rows()` 每次重建展开集合并遍历树；有关键字时对每个节点做匹配和拼音（`search.rs:38` 每个字符分配读音列表）；每个目录行收集一遍子孙算接口数；每个接口行还建按钮、提示、右键菜单和拖放。几百上千个接口、展开目录或在树里搜索时，打字、滚动、悬停都会卡。
- 修复方向：行缓存在 `WorkspaceState` 里，只在树、关键字、展开集合变化时重算；用 `uniform_list` 渲染（行高本来就固定，拖放落点用 行号 × 行高 算）。
- 修复：`WorkspaceState::rows()` 返回 `Rc<[Row]>`，按（树的构建次数、关键字、展开的目录）缓存，树每次重建经 `build_tree` 计数。树面板换成 `uniform_list`，每帧只生成看得到的行（目录的接口数、拼音高亮也只对这些行算）；滚动句柄换成 `UniformListScrollHandle`，拖放落点仍是“偏移 ÷ 行高”，拖到末尾的那条线改成按行数定位的覆盖层，滚到选中项改用列表自己的延迟滚动（不再需要等下一帧）。测试 `the_rows_are_made_once_for_what_they_show`：没变时是同一份行，树、展开、关键字变了才重做；树的构建不计数时它失败。**界面没有在真实应用里看过**：滚动、拖放（含末尾的线）、打开项目时滚到选中项、行下方空白处的右键菜单、重命名，需要手动确认一遍。

### PERF-2 GraphQL 响应每帧整份解析（高，GraphQL 用户）——已修复

- 位置：`src/views/workspace/response_panel.rs:1064-1065`（`render` 里 `graphql::response_errors(&response.body)`，整个响应体 `serde_json::from_slice`，新响应最多 64 MB）；`src/views/workspace/request_editor.rs:891-905`（`mark_graphql_errors` 在 `ApiState` 每次通知时执行，先解析 `GraphQlBody` 和 `response_error_places`，之后才比较缓存的 `key`，缓存没省下解析）。
- 修复方向：每个响应 id 只算一次（放在 `ApiState` 里，和 `checked_json` 一起）；`mark_graphql_errors` 先算 key 再解析。
- 修复：`ApiState::graphql_errors(response)` 按响应 id 缓存错误文字和位置，用断言共用的那份已解析的 JSON（`json_of`），core 多了不再解析的 `graphql::errors_in` / `error_places_in`。响应区的错误条和查询编辑器的标记都用它；`mark_graphql_errors` 先比较（响应 id，查询）再取位置。测试 `a_graphql_answers_errors_are_read_once`；让缓存永不命中时它失败。

### PERF-3 大响应先整份处理再截断，且在 UI 线程上（高）——部分修复

- 位置：`response_panel.rs:503-572`（`sync`）→ `editor_text`（`:1924-1940`）→ `body_text()`（`core/response.rs:424-428`：`pretty_json` 解析并重排整个响应体，然后才 `clip_text` 截到显示的 1–2 MB）；原始视图 `raw_body_text` 拷贝整个响应体；`sync_wire`（`:1609`）用完整响应体拼 `exchange()` 再截；树视图（`:496-497`，`json_tree.rs:154-161`）在主线程上解析最多 16 MB；`ApiState::checks()` / `json_of`（`states/api.rs:696、756-765`）第一次在 `render` 里解析；`EventStream::show`（`event_stream.rs:68`）解析全部事件；`views/workspace.rs:658-679` 每次切换选中都新建 `ResponsePanel`，以上全部重来。
- 情况：几 MB 到 64 MB 的响应，每次发送、每次切回这个标签，窗口卡几百毫秒到几秒。
- 修复方向：显示用的文本在后台生成，只处理截断后的前缀（超过上限时显示原文）；按响应 id 缓存在 `ApiState` 里，切标签时复用。
- 已修复的部分：Pretty / 原始视图和 Wire 视图只读显示得下的那一段。`HttpResponse::shown_body_text(pretty, limit)`：不超过上限的 JSON 照旧整份格式化；超过上限的由 `format::json_prefix` 逐字节重新缩进，到上限就停（数字和字符串保持原样，其余与整份格式化的开头一致，JSON lines 一行一个）；原始视图和文本用 `format::text_prefix` 只取前缀（在字符边界截断）。`exchange_within` 同理只取响应体放得下的部分。于是 64 MB 的响应每次显示、每次切回标签，工作量都是上限（1–2 MB）而不是整个响应体，不再需要缓存。测试 `a_json_bodys_start_is_indented_as_the_whole`、`a_texts_start_is_cut_at_a_character`、`a_bodys_start_is_shown_as_the_whole_would_be`（前缀与整份结果的开头一致）；去掉空容器的规则时第一条失败。
- 还没做的：树视图在主线程上解析最多 16 MB（`json_tree.rs`）、断言第一次在 `render` 里解析响应体、`EventStream::show` 解析全部事件；编辑器自己排版上限以内的文本（1–2 MB）仍在主线程上，这是显示上限的设置。

### PERF-4 首页轮询统计全库接口数（高，bridge）——已修复

- 位置：`crates/cyberapi-store-sql/src/scoped.rs:586-602` 执行 `API_COUNTS`（`sql.rs:250`：`SELECT collection, COUNT(*) FROM apis GROUP BY collection`，所有账号的所有项目），再在 Rust 里只留看得到的；由 `states/dashboard.rs:380` 的 `reload_listed` 调用，`poll`（`:148-160`）在网页上每 10 秒一次。页面这边 `read_drafts()`（`dashboard.rs:245-247`，`drafts_local.rs:70-83`）每次轮询都重读并解析全部 localStorage 草稿（每份是整个项目）。
- 情况：数据库负载随（全部接口行数 × 打开着的首页数）增长。
- 修复方向：SQL 加 `WHERE collection IN (看得到的)` 或和可见性查询 join；接口数比列表刷新得慢一些；草稿只在写入后重读。
- 修复：`API_COUNTS` 只统计账号经团队、账号授权、团队授权能到达的项目（和 `COLLECTIONS_VISIBLE` 同样的三条路，子查询 `IN (… UNION …)`，PostgreSQL 和 MySQL 都能用，走 `apis_collection` 索引），角色的精确判断仍在 Rust 里按 `visible()` 过滤。页面的草稿按文本的哈希记下读过的结果（`drafts_local::all` 返回 `Rc<Draft>`），文本没变就不再解析；仍然每次轮询读 localStorage，所以别的标签页写的草稿照样能看到。接口数的刷新频率没改。测试：`access.rs` 加了账号授权和团队授权两种情况（`make test-sql`，PostgreSQL 和 MySQL 都跑了；去掉账号授权那一路时两个库都失败），`a_draft_as_it_was_is_not_read_again`（让缓存不命中时失败）。

### PERF-5 运行弹窗每帧克隆并渲染全部结果（中高）

- 位置：`src/views/workspace/run_dialog.rs:284-289`（`results().to_vec()`，每条一个行元素），`:393-399`（一个滚动 `div`）。每个 `StepResult` 的 `HttpResponse::summary` 还带着完整的 `HttpRequest` 和响应头。
- 情况：带数据的运行（CSV 行数 × 接口数，成千上万步）越跑越卡。
- 修复方向：`uniform_list` 按下标渲染，借用而不克隆。

### PERF-6 打开着的流一直每 100ms 重绘，收发记录无上限（中高）

- 位置：`request_editor.rs:1259-1273`（`sending_since()` 有值时每 100ms 通知；`ApiState::connect` 直到连接关闭都保持发送中，`states/api.rs:1137`）；每条 WebSocket 消息各自 `cx.notify()`（`:1092-1098`），没有事件流那样的 `LIVE_FRAME` 合批；每条消息存三份：`OpenSocket.messages`、`LiveStream.body`、`LiveStream.events`（`api.rs:179、260-264`）；没有类似 `RESPONSE_LIMIT` 的上限（那个只管握手，`cyberapi-http/src/socket.rs`）；gRPC 客户端流（`OpenCall.messages`）一样；`EventStream::render` 每帧扫一遍全部事件。
- 情况：WebSocket、SSE、gRPC 流开几分钟到几小时：没有消息时 CPU 也不低，内存一直涨。
- 修复方向：连上之后把计时降到每秒一次或停掉；消息按帧合批；记录按大小或条数设上限并说明被截断。

### PERF-7 bridge 同步 / 上传逐行写入（中高，大项目）

- 位置：`scoped.rs:1024-1186`（`replace`）：每个目录 / 接口两次归属查询、每个变量 / 环境一次（`:1064-1077`）；每个删掉的接口一次删除；每个目录插入 + 写设置；每个接口的请求 JSON 都重写，包括没变的（`:1125-1151`）；每个变量插入 + 写环境。约 3 ×（接口 + 目录）次往返。
- 情况：1000 个以上接口的项目同步或上传要好几秒（远程数据库更久），全程占一个连接并持有项目的 `FOR UPDATE` 锁。
- 修复方向：归属检查一次批量查询（`id = ANY($1) AND collection <> $2`）；内容没变的接口跳过；多行插入。

### PERF-8 每个标签把完整响应体留在内存（中，内存）

- 位置：`states/api.rs:1438-1439`：`self.responses.insert(0, Arc::new(response))` 留的是未截断的响应体（最多 64 MB），每个接口最多 `history_size`（5–100）条，最多 16 个标签。存储里只留 2 MB 以内的（`for_history`）。
- 修复方向：只有最新显示的那条留完整响应体，其余换成 `for_history()` 的形式。

### PERF-9 打开标签时读全部历史连同响应体（中）

- 位置：`ApiState::load_history`（`api.rs:527-536`）每次打开标签执行，`HistoryStore::list` 读 `doc`（每条最多 2 MB，最多 100 条）。桌面版 `store-turso/src/lib.rs:1523-1537` 在持有唯一连接的锁时反序列化全部，保存和读树都排在后面；bridge `sql.rs:341`（`RESPONSES_OF`）整份以 base64 JSON 发出，页面在 UI 线程上解析；`recent` 读 50 份完整文档再丢掉响应体（`scoped.rs:1611-1625`）。
- 修复方向：列表只取摘要（或截到显示上限的响应体），选中某条时再取它的响应体。

### PERF-10 请求编辑器每帧克隆请求、每次按键解析请求体（中）

- 位置：`render_bar`、`render_tabs`、`render_body` 各自 `self.api.read(cx).request().clone()`（`request_editor.rs:1276、1541、1867`），连同请求体、示例（每个最多 256 KB）、gRPC 描述符；`render_grpc`（`:669`）每帧 `GrpcBody::read`（含 base64 描述符的整个 JSON），`own_grpc_definitions`（`api.rs:1607-1618`）再解析一次；每次按键（`watch_body`，`:536-539`）`value().to_string()` → `set_body`（trim + 拷贝）→ `check_json` → `send::json_problem`（`core/send.rs:432-454`：拆行再拼接，整份 `serde_json::Value` 解析）。
- 修复方向：借用而不克隆；按请求体缓存解析后的 `GrpcBody`；`json_problem` 防抖或放到后台（只做语法检查即可）。

### PERF-11 命令面板读最多 50 个项目的全部内容（中）

- 位置：`views/workspace/palette.rs:140` 在主线程克隆当前项目的整个 `TreeData`；`states/palette.rs:94-99` 对最多 50 个其他项目 `tree().load`（含全部请求 JSON：桌面占着数据库连接，网页是 50 次 bridge 调用）；`found_in`（`:35-58`）是 O(接口数²)；`items()`（`views/palette.rs:191-247`）每次 `render` 对全部接口 `format!` 加拼音匹配（`:377`）。
- 修复方向：只返回 id / 名称 / 方法 / 目录的存储调用；`found_in` 用 `HashMap`；按查询缓存过滤结果。

### PERF-12 压测每 300ms 排序全部样本（中）——暂不处理（维护者：压测暂不优化）

- 位置：`states/load.rs:176-181` → `core/load.rs:139-177`：每次收集并排序全部耗时、克隆每个错误字符串。按时长的测试最多留 100 万个样本（70 MB 以上），接近上限时每 300ms 在主线程上花几十毫秒以上。
- 修复方向：百分位用增量直方图（如按对数分桶），或在后台汇总；错误按种类计数，不留每次的字符串。

### PERF-13 网页版每次修改都把整个项目写进 localStorage（中）

- 位置：`states/copy.rs:154-176`（`persist`：`working.project()` → `serde_json::to_string` → `localStorage.setItem`），每次树的写入后调用（`:552-558`）；有仅本地条目时 `recount`（`:121-136`）还要克隆配置算指纹。都在页面线程上。
- 情况：大项目里每次停顿输入（300ms 保存）就整份序列化一次；还可能撞上约 5 MB 的 localStorage 上限。
- 修复方向：合并到空闲时再写（防抖），或按条目存增量；指纹增量计算。

### PERF-14 PostgreSQL 的 `responses` 缺 `api` 索引（中，bridge）

- 位置：`crates/cyberapi-store-sql/migrations/postgres/0002_responses.sql`：只有 `(account, api, seq)` 和 `(collection)` 两个索引，外键 `api REFERENCES apis(id) ON DELETE CASCADE`。PostgreSQL 的级联对每个删掉的 `apis` 行执行 `WHERE api = $1`，没有以 `api` 开头的索引就是全表扫描；删一个 2000 个接口的项目约 2000 次扫描。MySQL 的 InnoDB 会自己给外键建索引，没有这个问题。
- 修复方向：PostgreSQL 新增一个迁移 `CREATE INDEX responses_api_only ON responses (api)`；MySQL 那边对应一个空步骤（两套迁移逐列对应）。

### PERF-15 bridge `/v1/send` 没有全局内存上限（低中）

- 位置：`cyberapi-bridge/src/lib.rs:600-619`，`in_flight.rs:12`（`PER_ACCOUNT = 16`）。
- 情况：每个请求可能拿着 64 MB 响应体加它的 base64 JSON 应答（峰值约 150 MB），只有按账号的并发数限制，多个账号同时用时总内存没有上限。页面再在 UI 线程上解析这份应答。
- 修复方向：在按账号的计数旁边加一个全局信号量（或按字节的预算）。

### PERF-16 版本对比在 UI 线程上算差异（G8，低）

- 位置：`versions_dialog.rs` 的 `toggle_request` 同步调用 `diff::compare`；响应对比（`compare_dialog.rs`）是放到 `background_spawn` 的。请求体很大时展开一个接口会卡一下。
- 修复方向：同响应对比，放到后台。

## 要先定做法的

- **Q-1 来源地址自动获取（B7）**：`ProjectSettings.source.url` 是同步的项目数据，任何编辑者都能设（包括直接经 `/v1/store` 的 `set_project_settings`）。别的编辑者在桌面版打开“从来源同步…”时立即发一个 GET（跟随重定向，桌面引擎不限地址），可以指向本机或内网；能解析时预览里出现接口名，点同步就合并进共享项目。导入弹窗的“获取”用的是读者自己输入的地址，这里是别人定的、不用点击。建议：打开弹窗不自动获取，先显示地址、点“获取”才取；地址是本机 / 内网时提示。
- **Q-2 “来源里已经没有”的范围（B7）**：`import::merge::preview`（`merge.rs:150-163`）拿整个项目和来源比，一个来源在“spec”目录、另有 40 个手写接口的项目会被告知这 40 个“来源里没有”。另外同键的重复接口都算“不变”，而合并只更新第一个；预览按 `tree_data()` 算，同步时 `import_again` 用的是打开着的编辑器里未保存的请求，两边数字可能不一致。建议：只比较来源目录（`ImportSource.folder`）下的接口。
- **Q-3 重新导入来源地址时来源目录被改（B7）**：来源原来导入到“spec”，读者从项目菜单（目标是顶层）再导入同一个地址：`source_choice()` 不显示复选框（地址相同），但 `remember` 默认勾着，`import_settings` 写下 `ImportSource { url, folder: None }`，之后同步的新接口进顶层。建议：地址相同时保留原来的目录，或在弹窗里说明。
- **Q-4 目录 / 接口变量有模板提示但不求值（D5）**：`settings_dialogs.rs:128`、`request_settings.rs:76` 的键值编辑器都挂了 `.hints(hints)`，会提示 `{{value(..)}}`、`{{env(..)}}`，而这些值按原样使用（D5 的决定）。选了提示里的函数，发出去的是字面的 `{{…}}`。建议：去掉这两处的提示，或者改成对这些值求值（要看是否会循环引用）。
- **Q-5 压测发的是保存了的请求（A7）**（暂不处理：压测暂不优化）：`states/load.rs:56-65` 读 `tree_data()`；查看者的本地修改从不保存，压测发的是服务器上的版本，普通发送用的却是编辑过的；还在 200–300ms 防抖里的修改也不在。建议：标签打开着时用 `kept_api` 的请求（先 `flush`），否则用树里的。

## 看过、没有发现问题的

- D6：MCP `send_api` 的拒绝在 `in_flight` 之前，按所选环境的标记和 `asks_first` 判断；编辑器、WebSocket / gRPC 流的连接、运行器都会询问，已连接后发消息不问。
- D5：`Inherited::values` 的优先级正确；所有 `prepare` 的调用方（发送、代码、内省、反射、socket、gRPC 流、graphql-ws、OAuth、运行器、命令行、MCP）都带上了 `inherited` 和请求自己的变量；MCP `get_api` 对新字段打码。
- 存储：新的设置字段在 Turso 和 SQL 里都在 JSON 设置列中；SQL 的 `replace` 保留变量和环境的 id，G8 按 id 比较成立。
- F5：移动只在导入成功后才删除源。
- A6：`jsonschema` 关掉默认 feature，依赖树里不会取文件或网络上的 `$ref`，`$ref` 循环有处理；备份里没有 prefs、秘密值和 OAuth 令牌，复制不带秘密值。
- F6：备份的 `.part` 临时文件不会被 `made_at` 认成备份；wasm 下的备份函数都有空实现。
- B8：识别不会抢走 JSON（首行 `{` / `[` 不算）、YAML、Bruno、cURL 和单独一个地址；CRLF、请求头值里的冒号、`HTTP/x` 后缀都处理了；没有按字节切 `&str` 导致 panic 的地方。
- A7：按次数时正好发 N 次；并发是真的（每次发送是引擎里单独的 tokio 任务）；停止和关弹窗会丢掉任务链，发送随之取消；`inherited`、`secrets`、`environment` 都填了；生产环境确认和 prompt 都有。
- F7：释放顺序安全（`on_release` 时编辑器还在，更新在副作用处理之后，不会重入）；`Open` 里的窗口句柄在释放时移除；标签上限跳过独立窗口的标签；临时请求、删除、关闭都经 `join` 清理；移出前主窗口的编辑器先 `flush`。
- 性能方面已经做对的：响应对比和版本列表在后台、有行数上限、用 `uniform_list`；事件列表用 `uniform_list`，SSE 进度按 `LIVE_FRAME` 合批；JSONPath 过滤在后台；断言结果按响应 id 缓存；轮询的节奏、后台标签页放慢、失败退避都合理；导入弹窗的上限有效；SSE / 服务端流的响应体受引擎上限；`InFlight` 和 MCP 的 `Limiter` 会清理。
- 小问题，暂不处理：prefs 里的 `tabs` / `filters` 不清理已删除的接口（量很小）；桌面版最小化时仍每 30 秒轮询一次（一个很轻的请求）。
