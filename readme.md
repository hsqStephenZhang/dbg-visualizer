# Rust debugger visualizer

通过 `#[derive(dbgvis::Visualize)]` 在 Rust 内递归格式化逻辑值；普通字段优先 Visualize，再回退 Debug、Display。HashMap/Vec 等使用公共迭代接口，不解析内部存储，不需要用户编写分页。

非泛型 derive 同时自动登记根入口，由 linkme 跨 crate 收集；main 只需 `dbgvis::enable!()`，不再需要集中式注册模块。只复用 Debug 的类型用 `#[dbgvis::register]`；第三方或泛型具体实例用 `dbgvis::register_type!(T)`。类型级 `#[dbgvis(no_register)]` 可关闭自动登记。

多个 crate 登记同一具体类型时，runtime 比较具体类型 TypeId 后合并模式和函数入口。借用根以自由生命周期替换为 `'static` 的同一类型作为身份，formatter 仍保持生命周期泛化。同名但身份不同的类型即使 size/align 一致也会拒绝；名称和布局不充当类型身份。

当前为 **nightly specialization + GDB v2**，不保留 v1/LLDB 兼容层。[examples/explicit.rs](examples/explicit.rs) 演示 derive Visualize 和显式登记；[examples/demo.rs](examples/demo.rs) 保留 Debug 类型，通过实验 driver 自动登记。两个示例都无 feature gate；运行时仍默认 `native/manual`，加载脚本不自动调用目标函数。生产项目的可选 feature 接入方式见使用指南。

## 快速开始

```sh
cargo build --example explicit
rust-gdb -iex "add-auto-load-safe-path /absolute/path/dbg-visualizer/target/debug/examples/explicit" target/debug/examples/explicit
```

将 safe-path 替换为实际二进制绝对路径，不使用 `*`。GDB 脚本由 runtime 自带并嵌入产物，消费项目不需要自己的 build.rs 或 Python 副本。

```text
break explicit::checkpoint
run
up
dbgvis print app
dbgvis print map
dbgvis print external_map
dbgvis print --mode display address
dbgvis print --buffer 8 map
dbgvis config summary.mode auto
dbgvis config execution automatic
print point
```

示例 `map` 输出：

```text
{"points": [Some(Point { x: 1, y: 2 }), None]}
```

自包含 AppState 演示第三方字段、skip 和嵌套容器。另有三个 IndexMap 实例、hashbrown::HashMap、Bytes、SocketAddr、借用/const 泛型及无 Debug 的 ZST hasher。跨 crate 和 feature 开关验证由测试运行时生成临时项目，workspace 只保留三个核心 crate 加一个 xtask 验证入口。

若要运行无需手写根登记的 `demo`，在仓库根目录执行（driver 要求 `rustc 1.99.0-nightly (12c36e253 2026-08-10)`）：

```sh
cargo xtask driver
DBGVIS_AUTO_CRATE=demo RUSTC_WORKSPACE_WRAPPER="$PWD/tools/auto-register/wrapper.py" CARGO_TARGET_DIR=target/auto-register/examples cargo build --example demo
rust-gdb -iex "add-auto-load-safe-path /absolute/path/dbg-visualizer/target/auto-register/examples/debug/examples/demo" target/auto-register/examples/debug/examples/demo
```

断点为 `demo::checkpoint`，`run`、`up` 后用 `dbgvis p map` 或 `dbgvis p index_borrowed`。普通 `cargo build --example demo` 不启用 driver，不能据此期待这些根已登记。driver 默认只扫可执行目标自身；对「逻辑在 lib、main 只是入口」的布局，需要 `DBGVIS_SCAN_DEPS=1` 才能扫到库里的局部变量，代价是给 workspace 库加 `-Zalways-encode-mir` 且 strict 失败面变大。细节见[两阶段自动注册实验](docs/自动注册可行性验证.md)。`dbgvis p -m native "hello"` 保留表达式引号；`--` 可结束打印选项。完整命令见 `dbgvis help`。

## 文档

- [使用指南](docs/使用指南.md)：接入 API、feature 开关、GDB 命令与安全边界。
- [协议 v2](docs/协议-v2.md)：注册、mailbox 与有界文本。
- [第二阶段验收记录](docs/第二阶段验收记录.md)：实际测试、已知限制和未完成项。
- [Q0 技术验证](docs/archive/Q0技术验证.md)：为何采用 nightly（归档，工具链决策已定）。
- [两阶段自动注册实验](docs/自动注册可行性验证.md)：无需逐类型手写登记的 driver 原型、验证方法与已知限制（可选，不改变默认构建）。

## 验证

```sh
cargo fetch --locked
cargo fmt --all -- --check
cargo clippy --offline --workspace --all-targets --all-features -- -D warnings
cargo test --offline --workspace --all-features
cargo xtask compiler
cargo xtask integration --faults --lto --relocated
cargo xtask autoregister --gdb
```

当前验证环境为 Linux x86_64、GDB 17.1、rustc 1.99.0-nightly (12c36e253 2026-08-10)。集成测试需要本地 ptrace 权限；在 macOS 等非 Linux 平台上该套件直接跳过（Apple/MSVC 目标不生成 `.debug_gdb_scripts`），其余四步照常运行。调试构建必须保留 `debug = 2` 且不 strip，否则嵌入脚本与 anchor 类型信息都会消失。缺少全部格式化能力的自动分派实例需 `cargo build` 才保证诊断，不能仅依赖 `cargo check`。

有界缓冲区不保证用户 fmt 纯、无分配或不阻塞。core dump 和无法可靠定位的值只能走 native。核心已经实现；`cargo-dbgvis setup/doctor/uninstall` 与完整发布演练尚未实现，crate 尚未发布。
