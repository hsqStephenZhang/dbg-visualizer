# Rust debugger visualizer

通过一次类型注册，在 GDB、LLDB 和 CodeLLDB 中选择调试器原生视图，或使用 Rust 类型自己的 `Debug` / `Display` 输出。格式化直接写入有界缓冲区；容器支持按需分页展开。

默认配置为 `native` 和 `execution=manual`，加载脚本不会自动运行 Rust 格式化函数。显式 `dbgvis print` 默认使用 Debug；开启 automatic 后，变量窗口使用所选模式。

## 快速开始

```sh
cargo build
rust-gdb -iex "add-auto-load-safe-path $(pwd)/target/debug/dbg-visualizer" target/debug/dbg-visualizer
```

在 GDB 中：

```text
break dbg_visualizer::checkpoint
run
up
dbgvis print bytes
dbgvis print --mode display point
dbgvis print --buffer 8 map
dbgvis page --start 0 --count 2 map
dbgvis config summary.mode debug
dbgvis config execution automatic
dbgvis config children.mode structured
print map
```

在 LLDB 中：

```sh
lldb target/debug/dbg-visualizer
```

```text
command source .lldbinit
dbgvis print --mode display point
dbgvis config summary.mode debug
dbgvis config execution automatic
dbgvis config children.mode structured
frame variable map
```

VS Code 使用仓库的 `.vscode/launch.json`。在 `checkpoint` 停下后选择调用栈中的 `main` 帧，调试控制台使用同样的 `dbgvis` 命令。

## 文档

- [使用与注册指南](docs/使用指南.md)：配置、加载、注册新类型、容器适配和限制。
- [协议 v1](docs/协议-v1.md)：入口、布局、缓冲区和错误码。
- [实现计划](docs/实现计划.md)：原始需求及实施阶段。
- [验收记录](docs/验收记录.md)：测试范围、支持矩阵和性能基线。

## 验证

```sh
cargo test --offline --workspace
cargo clippy --offline --workspace --all-targets -- -D warnings
python3 -m unittest discover -s tests -p 'test_*.py'
python3 tests/integration/build_profiles.py
python3 tests/integration/run.py --faults --core --benchmark
python3 tests/integration/codelldb.py --adapter /path/to/extension/adapter/codelldb
```

首次离线验证前运行 `cargo fetch --locked`。集成测试需要 Linux 本地 ptrace 权限；每个 debugger 子进程有外部超时限制。

当前支持 Linux x86_64、本地调试、单一可执行程序注册表。有界输出不保证用户 `fmt` 纯、无分配或不会阻塞。core dump 使用原生视图；完整支持范围见验收记录。
