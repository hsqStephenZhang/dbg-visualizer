# Rust debugger visualizer

通过 `#[derive(dbgvis::Visualize)]` 在 Rust 内递归格式化逻辑值；普通字段优先 Visualize，再回退 Debug、Display。HashMap/Vec 等使用公共迭代接口，不解析内部存储，不需要用户编写分页。

当前为 **nightly specialization + GDB v2**，不保留 v1/LLDB 兼容层。默认关闭项目 `visualize` feature；启用后仍默认 `native/manual`，加载脚本不自动调用目标函数。

## 快速开始

```sh
cargo build --features visualize
rust-gdb -iex "add-auto-load-safe-path /absolute/path/dbg-visualizer/target/debug/dbg-visualizer" target/debug/dbg-visualizer
```

将 safe-path 替换为实际二进制绝对路径，不使用 `*`。GDB 脚本由 runtime 自带并嵌入产物，消费项目不需要自己的 build.rs 或 Python 副本。

```text
break dbg_visualizer::checkpoint
run
up
dbgvis print app
dbgvis print map
dbgvis print external_map
dbgvis print --mode display point
dbgvis print --buffer 8 map
dbgvis config summary.mode auto
dbgvis config execution automatic
print point
```

示例 `map` 输出：

```text
{"points": [Some(Point { x: 1, y: 2 }), None]}
```

AppState 演示 a/b/c 三个库的组合、非 Debug 类型、第三方 Debug/Display 字段、skip 和嵌套容器。另有三个 IndexMap 实例、hashbrown::HashMap、Bytes、SocketAddr、借用/const 泛型及无 Debug 的 ZST hasher。

## 文档

- [使用指南](docs/使用指南.md)：接入 API、feature 开关、GDB 命令与安全边界。
- [实现计划](docs/实现计划.md)：已批准路线及安装分发等后续任务。
- [协议 v2](docs/协议-v2.md)：注册、mailbox 与有界文本。
- [第二阶段验收记录](docs/第二阶段验收记录.md)：实际测试、已知限制和未完成项。
- [Q0 技术验证](docs/Q0技术验证.md)：为何采用 nightly；第一阶段文档只作归档。

## 验证

```sh
cargo fetch --locked
cargo fmt --all -- --check
cargo clippy --offline --workspace --all-targets --all-features -- -D warnings
cargo test --offline --workspace --all-features
python3 tests/compiler/run.py
python3 tests/integration/run.py --faults --lto --relocated
```

当前验证环境为 Linux x86_64、GDB 17.1、rustc 1.99.0-nightly (12c36e253 2026-08-10)。集成测试需要本地 ptrace 权限。缺少全部格式化能力的自动分派实例需 `cargo build` 才保证诊断，不能仅依赖 `cargo check`。

有界缓冲区不保证用户 fmt 纯、无分配或不阻塞。core dump 和无法可靠定位的值只能走 native。核心已经实现；`cargo-dbgvis setup/doctor/uninstall` 与完整发布演练尚未实现，crate 尚未发布。
