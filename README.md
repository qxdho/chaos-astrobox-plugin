# Chaos Connect（AstroBox v4）

适配 Chaos 原生文件桥的新私有协议 **ChaosConnect v1**。插件0.3.2不再调用 CanopusConnect，不需要模块管理器；必须同时更新 Chaos 与插件，旧 band_files 插件不兼容。

## 使用

1. 将配套 Chaos 安装包安装到小米手环10 Pro / 3.101.043，先重启设备，再运行/注入新版Chaos，以释放旧驻留文件桥的处理入口。
2. 在 Chaos 设置 → ChaosConnect 中打开“启用”（原开关配置保留，默认关闭）。
3. 在支持通用设备传输的 AstroBox v4 中安装 `dist/chaos-connect.abp`，连接设备后打开插件，先点击“刷新设备”，再点击“连接ChaosConnect”。插件启用时不自动查询设备或发送握手。
4. 从 `/data` 浏览，支持上传、下载、目录、详情、复制/移动、删除、预览和编辑；系统区域写入需要手动开启开发者模式。
5. 点击“链路测速（1 MiB）”，记录实际 KB/s / kb/s 以及是否达到200 kb/s，再通过实际文件传输核对闪存读写速度。

传输时不要切换设备；插件会阻止操作并在断线/应答超时后取消任务。上传先写同目录独有临时文件，所有数据块应答确认、CRC32校验与关闭成功后rename；失败不先删除原文件。默认写入 `/data`，开发者模式可开放其他实际路径，始终拒绝 `/dev`、`/proc`、`/sys`。单文件上传上限32MiB，文本预览上限256KiB。

## 承载与限制

```text
插件 transport.send / register_transport_recv
  → AstroBox认证设备会话（小米SARv2 / L2 PB通道）
    → WearPacket.Type.OTHER (0) / ChaosConnect CCN1
      → Chaos原生文件操作
```

不再使用 CPC2 / CanopusConnect / ConnectListen / ConnectSend / 模块名称RPC。权限只有 `request`、`register_transport_recv`、`device`，组件也没有 CanopusConnect import/export。

**这是独立应用协议，未新增裸 Bluetooth L2CAP PSM。** 官方v4插件API没有裸L2CAP socket入口，实际蓝牙承载仍由AstroBox选择。插件使用官方已有 `transport` / `register` 能力，不依赖虚构的新宿主接口。较旧AstroBox中这些接口可能仍是占位实现，需要升级到已实现通用设备传输的版本。

已核对 [v4文档](https://abox.run/docs/plugin-development/v4) 及共享开发文档、[最新宿主发送实现](https://github.com/AstralSightStudios/AstroBox-NG-Module-PluginSystem/blob/main/src/v4/host/transport.rs) 和 [事件分发](https://github.com/AstralSightStudios/AstroBox-NG-Module-PluginSystem/blob/main/src/manager.rs)。文档的transport页仍保留早期占位描述，因此也核对了实际宿主源码。

数据块上限2048B、窗口8，按固件SAR协商容量下调；不逐块等待RPC往返。接收事件用Base64解码、校验私有namespace、CRC及连接前缀，再串行处理，防止异步下载写入乱序。上传根据设备ACK推进，所有在途数据确认后才提交。

200 **kb/s** 按标准小写单位为25,000 B/s，与200 **KB/s**不同。测速使用实际1MiB收发、回显数据校验与单调时钟，显示单方向有效速度；不通过主机CPU循环、虚构帧率或理论带宽宣称达标。**当前只有编译/主机/ARM验证，尚未真机确认200 kb/s。** 回显不写手环文件，也不代表闪存文件传输性能。

## 构建与验证

需要Rust、`wasm32-wasip2`、zip：

```sh
rustup target add wasm32-wasip2
cargo test --manifest-path plugin/Cargo.toml --lib
sh scripts/build_plugin.sh
```

`dist/chaos-connect.abp` 包含manifest、图标和 `astrobox_plugin_chaos_connect.wasm`。manifest遵守API Level4 / WASI3，组件使用宿主同时支持的WASI p2标准库。

测试覆盖CCN1帧边界/CRC、路径与解析、所有ACK后提交，以及按真实WearPacket/MessageContent字段布局通过prost解码再编码，确认合法外层type和二进制内容不会被宿主丢弃；通过宿主同版本pbjson生成器验证JSON日志序列化（包含旧17219错误复现、HELLO和最大数据块），并验证on_load不调用设备/传输接口。Chaos仓库另有生产C协议/真实临时文件系统测试与精确043固件ARM探针。

协议字段、文件操作和速度验收见 [docs/PROTOCOL.md](docs/PROTOCOL.md)。

## 0.2.1 启用崩溃修复

0.2.0 的外层私有枚举 `0x4343` 虽能通过prost二进制编解码，但宿主PB发送日志中的 `serde_json::to_string(...).unwrap()` 会因pbjson不认识该枚举而panic（`Invalid variant 17219`），结束ECS核心线程。之后看到的 `runtime thread has stopped: SendError` 是连带错误，重复启用插件不能恢复。

0.2.1改用schema已有的 `OTHER=0`，私有身份继续由 `ChaosConnect` 包名、CCN1和CRC校验；两端必须一起更新。启用时只完成生命周期回调，用户手动刷新设备和连接。更新插件与Chaos后，需要**完全退出并重启AstroBox**以恢复已退出的核心线程，然后按上方步骤连接；设备端也需重启并注入新版Chaos。

## 0.2.2 状态与手环授权

需要同时更新Chaos。HELLO检查状态/权限能力位0x04，连接后每3秒PING，等待授权时每秒同步手环实际状态。插件心跳10秒无响应则显示断开；设备15秒没有有效请求则关闭文件、清理未提交上传并撤销开发者模式。

手环“ChaosConnect → 权限设置 → 允许开发者模式”可设开启/关闭/询问，默认询问。请求开启时插件显示等待手环确认，并提供取消按钮；手环将亮屏并震动，请在原生弹窗选择取消或允许。60秒内未确认自动拒绝。授权只在当前连接有效，状态以手环结果为准，关闭权限会立即撤销；插件不能通过文件协议改写手环权限配置。正常PING不会淹没操作日志。

## 0.2.3 手环停用即时断开

配套Chaos关闭“启用”时立即封锁协议，并通过设备状态事件通知插件。插件收到匹配当前连接前缀且CRC有效的停用事件后立即清除连接、开发者模式、授权等待与文件任务；重新打开需点击重新连接。无线通知丢失仍由原心跳超时收尾。请同时更新两端，并重启手环后注入新版Chaos以释放旧驻留入口。

## 0.2.4 显示信息

插件显示名称为“Chaos Connect”，作者`IKUN_CXKPRO`，图标直接使用Chaos原始`tools/chaos.png`。测速结果只显示`KB/s · kb/s`，不再显示是否达到200kb/s。配套Chaos修复日志视口偏移、状态主副标题刷新，并在新授权请求时亮屏和震动一次。

## 0.3.0 NSH终端

连接后选择 **NSH终端**，点击“启动终端”。自0.3.2起插件NSH不再需要独立权限，仍需手环允许本次ChaosConnect连接。命令直接连接手环独立原生 PTY，不依赖开发者模式；支持 UTF-8 输出、命令发送、Ctrl+C、停止和清空输出。返回文件管理可继续使用原文件功能。

按官方 [v4异步接口](https://abox.run/docs/plugin-development/v4/whats-new) 和现有 UI／Timer／Transport API 接入，无新增宿主权限。终端采用 `0x50..0x53` 与 lease，旧应答／旧会话不能推进新命令；命令不在不确定结果后自动重发。撤权和断连停止访问。最多保留32KiB文字输出，常见ANSI控制序列被过滤，不是完整VT100。可见时约250ms读取，回到文件管理后最多每秒一次；空输出不重复刷新整个页面。

`cargo test --manifest-path plugin/Cargo.toml --lib` 20项通过；`sh scripts/build_plugin.sh` 产出 `dist/chaos-connect.abp`。已验证解析、lease、撤权、分段UTF-8及有界输出，尚未在真实AstroBox与手环上执行命令。需要同时使用支持NSH协议的Chaos版本。


## 0.3.1 NSH连接与输入修复

启动时手环可能仍在回收上一轮PTY。插件现在将OPEN的BUSY作为可等待状态，每500ms重试，最多15秒，界面显示回收等待及超时原因；不再直接报文件操作的“目录非空或设备忙”。权限拒绝等错误不会重试。

修复输入框每个字符触发整页重绘造成的卡顿与光标中断：输入事件只保存草稿；聚焦时暂缓整页渲染，NSH输出仍在后台读取并缓存，失焦后再刷新。JSON输入用标准解析器处理中文、引号、反斜杠和Unicode转义。

发送和Ctrl+C通过有界队列进入NSH。正在读取输出时仍可点击发送，READ完成后优先发送命令，发送按钮不再跟随READ忙状态禁用。只有收到明确BUSY（设备未接收任何字节）时才间隔重试写入，最多5秒；已确认写入或发送结果不确定时均不重发。命令队列最多4条，过长／无效输入会保留草稿提示。

需更新插件 `dist/chaos-connect.abp`，可继续使用现有配套Chaos模块。此修复不改变手环PTY回收的所有权规则。33项Rust回归及WASM构建通过；未在本轮操作实体手环。


## 0.3.2 取消插件NSH独立权限与原生输入

配套Chaos移除“插件权限 → NSH终端”开关和插件mask 0x20的终端门禁。连接允许后即可启动NSH；Android端NSH权限保持独立，仍由手环设置。请同时更新Chaos模块与本插件。

为解决逐字跨WASM/宿主事件的卡顿，终端现在使用“输入命令／修改命令”按钮打开宿主原生输入弹窗。文字直接在宿主编辑，确认后一次返回插件，再点“发送”。取消弹窗保留原草稿；不再注册NSH逐字输入与焦点事件。编辑期间不重建插件页面，后台仍读取并缓存输出。原有待发送队列、Ctrl+C、停止和超时处理继续使用。

34项插件回归与模块31项生产协议回归通过，WASM/模块构建通过。本轮未操作实体设备，原生输入交互和真机延迟仍需复测。
