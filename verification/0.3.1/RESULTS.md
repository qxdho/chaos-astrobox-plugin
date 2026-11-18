# 0.3.1 NSH连接、编辑与发送修复

- `cargo test --manifest-path plugin/Cargo.toml --lib`：33项通过，无失败和跳过。
- `sh scripts/build_plugin.sh`：wasm32-wasip2 release构建成功，ABP中的manifest和WASM与dist文件一致。
- 根因：OPEN返回暂忙被当作终止错误；输入每字触发整页render；READ期间禁用发送按钮且点击被丢弃；手工JSON解析丢失反斜杠／Unicode转义。
- 修复：OPEN暂忙间隔500ms重试、最多15秒；待发送队列与READ独立；当前读应答完成后优先发送；编辑期间保存草稿并延迟整页绘制，失焦350ms后刷新；标准JSON解码。
- 明确WRITE BUSY表示模块TX.push_all没有接收数据，最多等待5秒后提示重新输入；已确认写入、超时、链路错误均不自动重发。
- 回归覆盖启动忙后恢复、等待上限、权限拒绝不重试、停止后晚到OPEN回收、在途READ不影响发送、队列上限、Ctrl+C顺序、发送确认只移除一次、暂忙与不确定错误的不同处理、编辑焦点、UTF-8／ANSI／历史上限、旧lease及撤权。
- 保持现有模块ABI和NSH lease协议，无需更换手环模块；没有放弃或释放未回收的PTY。
- 本轮没有操作实体手环或真实AstroBox界面；以上为实际客户端状态机／协议测试与组件构建，不能代替真机交互验证。

产物：dist/chaos-connect.abp；SHA-256见results.json。
