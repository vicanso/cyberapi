# 依赖的本地补丁

这里是用根目录 `Cargo.toml` 的 `[patch.crates-io]` 替换进来的依赖副本：上游已经修了，但我们的依赖链还用不上那个版本。每个副本都不是工作区成员（`[workspace] exclude`），`make lint` / `cargo test --all` 不碰它们。上游可用之后删掉目录和 `[patch.crates-io]` 里的那一行，再 `cargo update -p <包名>`。

## accesskit_macos 0.26.3：带 Touch Bar 的 Mac 上关窗口崩溃

**现象**：带 Touch Bar 的 Mac（例如 MacBookPro17,1）上，关闭第二个窗口（设置窗口等）时应用退出，崩溃报告是

```text
*** Terminating app due to uncaught exception 'NSRangeException', reason: 'Cannot remove an observer
<_NSTouchBarFinderObservation 0x…> for the key path "nextResponder" from <NSView 0x…> because it is
not registered as an observer.'
  -[_NSTouchBarFinderObservation invalidate]
  ___NSTouchBarFinderSetNeedsUpdateOnMain_block_invoke_2
  NSDisplayCycleObserverInvoke
```

用 gpui-kit 的应用都有（zedis 也是，GPUI Fast 和 gpui-pre 一样）；没有 Touch Bar 的 Mac 不会出现。

**原因**：GPUI 的 macOS 后端用 `accesskit_macos::SubclassingAdapter` 给窗口的内容视图动态加一个子类（`AccessKitSubclassOf…`）。有 Touch Bar 时 AppKit 用 KVO 观察 key window 响应链上的视图（`nextResponder`），KVO 又在 AccessKit 的子类上面套了一层 `NSKVONotifying_…`。窗口关闭时 GPUI 释放适配器，0.26.3 的 `Drop` 无条件用 `object_setClass` 把视图换回原来的类，连 KVO 那层也拆掉了；Touch Bar 稍后（下一次更新时，不是固定的时刻）移除观察者时就抛异常。

**上游**：

- AccessKit [issue #823](https://github.com/accesskit/accesskit/issues/823)（同样的现象和分析）、[PR #835](https://github.com/accesskit/accesskit/pull/835)（修复，2026-10-23 合并）、发布在 **accesskit_macos 0.27.2**（2026-10-25）。0.26.x 没有补丁版。
- gpui-kit [issue #3192](https://github.com/longbridge/gpui-kit/issues/3192)、Zed [issue #64819](https://github.com/zed-industries/zed/issues/64819) / [PR #65186](https://github.com/zed-industries/zed/pull/65186)（改成不用动态子类）报的是同一个崩溃，截至 2026-10-26 都还开着。

**为什么不直接用 0.27.2**：gpui-fast-macos / gpui-pre-macos（截至 0.1.12 / 0.3.10，即 gpui-kit 0.8.1）要求 `accesskit_macos = "0.26"`、`accesskit = "0.24"`，而 0.27.2 依赖 accesskit 0.25。`[patch]` 不能放宽版本要求（指向 0.27.2 时 Cargo 提示 patch was not used）；把 0.27.2 改个版本号硬塞进来，GPUI 的 `TreeUpdate`、`ActivationHandler`、`ActionHandler` 是 0.24 的类型，编译不过。

**补丁的内容**：crates.io 上 0.26.3 原样的源码，只有 `src/subclass.rs` 换成 PR #835 的版本（PR 里 `adapters/macos/src/subclass.rs` 的改动原样应用，结果与 0.27.2 的 `src/subclass.rs` 逐字节相同；PR 里新增的测试没有带过来）。修复的做法：释放适配器时看视图真正的类（`object_getClass`，`[view class]` 会被 KVO 骗），被 KVO 再子类化过的话就不动类，只释放 AccessKit 的状态；子类上的方法发现状态已经没了，就转给原来的类处理。没有 KVO 时和原来一样换回类。

重新生成这份副本：

```sh
gh pr diff 835 --repo accesskit/accesskit \
  | awk '/^diff --git a\/adapters\/macos\/src\/subclass.rs/{p=1;print;next} /^diff --git/{p=0} p' > subclass.diff
# 在 accesskit_macos-0.26.3 的源码目录里：
patch -p3 < subclass.diff
```

**验证**：没有 Touch Bar 的 Mac 也能复现——Xcode 菜单 Window → Touch Bar → Show Touch Bar（⇧⌘8）打开 Touch Bar 模拟器后，其他应用都会被 AppKit 当作有 Touch Bar。

- 没有补丁：打开设置窗口再 ⌘W 关掉，第一两次就崩，异常与上面相同。
- 有补丁：开着模拟器开关设置窗口十次、⌘Q 正常退出（退出码 0）；关掉模拟器再开关三次，也正常（走原来换回类的路径）。
- `CYBERAPI_NO_ACCESSIBILITY=1` 启动（不挂 AccessKit）时没有补丁也不崩，说明问题只在 AccessKit 这一处。

**什么时候删**：gpui-kit 用的 GPUI 后端升级到 accesskit_macos ≥ 0.27.2（或像 Zed #65186 那样不再用 `SubclassingAdapter`）之后。删之前在模拟器里按上面的步骤再试一次。
