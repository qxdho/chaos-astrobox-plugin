# 0.3.0 NSH验证

- `cargo test --manifest-path plugin/Cargo.toml --lib`：20项通过，0失败。
- `sh scripts/build_plugin.sh`：wasm32-wasip2 release构建通过，API Level4 / WASI3。
- 终端测试覆盖分段UTF-8、ANSI过滤、32KiB上限、错误请求编号、旧lease、撤销NSH权限；原有帧／CRC、宿主protobuf再编码、启用不访问设备、开发者授权与文件传输提交测试继续通过。
- ABP中的manifest与WASM均与dist当前文件逐字节一致；作者和图标保持不变，新增独立NSH页面与定时读取，不添加宿主权限。
- 控制台支持命令、Ctrl+C、停止、清空输出，用户停止和超时不自动重发命令。关闭后停止定时器，空输出不重复整页渲染。
- 未在真实AstroBox与手环验证命令执行；手环生产C接口和PTY后端另有协议与生命周期回归。

产物及校验值见results.json。
