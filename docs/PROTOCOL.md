# ChaosConnect v1

私有命名空间 `ChaosConnect`，与旧插件不兼容。整数均为小端。

## CCN1帧

| 偏移 | 字段 |
|---|---|
| 0 | `CCN1`，4B magic |
| 4 | u8 version = 1 |
| 5 | u8 flags：0请求、1应答、2事件 |
| 6 | u8 opcode（应答保持请求opcode，不再OR 0x80） |
| 7 | u8 status，请求为0 |
| 8 | u16 request_id，事件为0 |
| 10 | u16 payload长度，最大2304 |
| 12 | u32 CRC32 |
| 16 | payload |

CRC32为标准IEEE多项式 `0xEDB88320`，初值和终值异或均为 `0xFFFFFFFF`，覆盖前12B头与payload（跳过CRC字段本身）。完整长度必须等于16+payload长度。拒绝旧CPC2、错误版本、截断、超长、重复目标protobuf字段、溢出varint和CRC错误。

## AstroBox承载

使用v4官方 `transport.send(addr, bytes)`，并通过 `register_transport_recv(addr, {channel_id:1, protobuf_typeid:0})` 订阅。权限字符串分别为 `request`、`register_transport_recv`，设备列表另需 `device`。

`protobuf_typeid=0` 在宿主订阅中表示通配，仍由 `channel_id=1` 限定PB通道；插件解包必须另外验证外层type恰为0、包名、CCN1、CRC、连接前缀和请求ID。普通PB会被忽略。宿主第三方消息组件还可能广播同一MessageContent，但不参与ChaosConnect应答处理。

外层type必须是schema已定义枚举。旧0.2.0使用17219，虽能通过prost却在宿主pbjson日志序列化时失败并触发unwrap panic。0.2.1使用OTHER，不改变CCN1内部帧。插件启用阶段不查询设备或自动HELLO，手动刷新设备和连接后再订阅/发包。

宿主对发送内容使用WearPacket解码再编码，必须使用已定义的字段保存二进制：

```text
WearPacket.type (field 1) = OTHER (0，schema中已定义)
WearPacket.id   (field 2) = connection_prefix:u16 << 16 | request_id:u16
WearPacket.thirdparty_app (field 22)
  message_content (field 9)
    basic_info (field 1)
      package_name (field 1) = "ChaosConnect"
      fingerprint (field 2) = empty bytes（required字段）
    content (field 2) = CCN1帧
```

消息进入已认证的SARv2/L2 PB承载。设备只拦截OTHER类型、ChaosConnect namespace和正确帧；正常PB、其他L2通道（包括0x43）原样转交系统。**这不是独立L2CAP PSM或裸L2CAP socket协议**；蓝牙具体承载由宿主决定。官方最新宿主源码已实现通用设备传输，旧占位宿主不支持。

接收的 `transport-packet` event-payload是**直接的标准Base64字符串**，解码后为WearPacket。应答完整回显ID；事件使用当前连接前缀+低16位0。插件拒绝旧连接前缀和ID不匹配的帧。重新连接先换前缀再HELLO；设备的新HELLO清理原读写事务和开发者模式。

## 路径、状态

路径编码为 `[length:u16][UTF-8 bytes]`，单路径不超过255B，无NUL，规整`.`、`..`和重复`/`。上传与复制的临时后缀还需额外空间，过长目标报错。默认只写 `/data`；开发者模式可开放其他实际路径，始终拒绝`/dev`、`/proc`、`/sys`。

状态：0成功；1参数；2不存在；3访问拒绝；4已存在；5非目录；6是目录；7空间不足；8忙/非空；9IO；10CRC；11偏移序列；12不支持；13过大；14等待手环授权。

## 操作

| opcode | 请求payload | 成功应答payload |
|---|---|---|
| 0x01 HELLO | 空 | version:u8, max_data:u16, flags:u8（bit2=状态/授权能力）, dev_mode:u8, max_path:u16, window:u32 |
| 0x02 LIST | path, offset:u32, limit:u8（1–16） | total:u16, offset:u16, count:u16, entries |
| 0x03 STAT | path | is_dir:u8, size:u32, mtime:u32 |
| 0x04 MKDIR | path | 空 |
| 0x05 REMOVE | path | 空 |
| 0x06 RMDIR | path（只删除空目录） | 空 |
| 0x07 RENAME | source_path, target_path | 空 |
| 0x08 COPY | source_path, target_path, overwrite:u8 | 空 |
| 0x10 OPEN_READ | path | size:u32 |
| 0x11 READ | offset:u32, length:u16 | 实际数据bytes |
| 0x12 CLOSE_READ | 空 | 空 |
| 0x20 OPEN_WRITE | path, expected_size:u32, overwrite:u8 | 空 |
| 0x21 WRITE | offset:u32, data | committed_offset:u32, cumulative_crc32:u32 |
| 0x22 COMMIT | expected_crc32:u32 | 空 |
| 0x23 ABORT | 空 | 空 |
| 0x30 PING | 空 | connected:u8, dev_mode:u8, policy:u8, authorization:u8, token:u32 |
| 0x31 BENCH_ECHO | 测试数据，长度≤max_data | 原样回显测试数据 |
| 0x40 SET_DEV | enabled:u8 | 立即完成时空；等待授权为status14 + token:u32 |
| 0x41 GET_DEV | 空 | enabled:u8 |

LIST每项：name_len:u8、name、is_dir:u8、size:u32、mtime:u32。目录帧也遵守协商容量；不得跳过放不下的大条目再打包后面的条目，否则offset会导致丢项。计数0但仍有条目时明确报TooLarge，不无限分页。

进度事件flags=2、opcode=0xC1，payload为written:u32、cumulative_crc32:u32。它只更新显示，不能代替每块WRITE ACK或COMMIT应答。

## 窗口、可靠性与200 kb/s验收

- max_data=`min(2048, SAR协商容量-128)`，窗口8。这里是SAR帧限额，不是Bluetooth L2CAP MTU。保留头部余量满足固件 `payload_len+1 < SAR_capacity` 检查。
- 上传数据块顺序入队，设备校验偏移并处理短写；插件记录每个request_id的offset/长度，可以容忍ACK乱序，但全部ACK确认后才能COMMIT。
- 上传先写同目录独有临时文件，再核对总大小/CRC、close、rename；覆盖失败不先删原文件。ABORT、新HELLO和设备会话结束清理临时文件。
- 下载允许8个READ在途，按偏移组装并串行await本地保存，防止并发回调导致落盘乱序；块长度必须精确符合协商值与文件剩余长度。
- 原生RX/TX队列分别12/16，处理前预留应答空间。发送在BT loop；原生发送会就地加密，因此每次重试从保留的明文复制，不二次加密旧密文。断连或开关停止推进epoch，旧帧不能污染新会话。
- 上传/下载应答超时15s取消；插件传输中禁止换设备。没有宣称断点续传。
- 1MiB链路回显测速使用真实传输和单调时钟，不写手环文件；显示单方向有效 `bytes/seconds`、KB/s、kb/s。200 kb/s=25,000 B/s，200 KB/s是另外一个目标。
- 编译、CPU测试、理论窗口带宽、ARM模拟均不能证明无线达标。当前200 kb/s尚待真机验证；回显测速与实际上传/下载需分别记录。

固定互通向量保存在`CHAOSCONNECT-V1.hex`（插件位于`plugin/tests/`）：第一行为HELLO请求CCN1帧（request_id=0x1234），第二行为同一帧的宿主protobuf承载（packet_id=0x12341234）。两仓库的向量逐字节相同，C与Rust测试分别核对。

## 设备状态和开发者模式授权（插件0.2.2）

HELLO flags bit2 (0x04)必须置位。PING状态固定8B；policy=0开启/1关闭/2询问；authorization=0无请求/1等待/2允许/3拒绝/4超时，非0状态必须携带非0 token。connected与dev_mode只能0/1。PING独立于文件请求的pending槽，插件匹配request_id和连接前缀后才消费应答。

SET_DEV(1)：policy开启时立即成功，关闭时Access，询问时Pending(14)+token，设备弹窗确认后通过后续PING同步结果。重复请求复用同一个待定token；60秒授权超时。SET_DEV(0)关闭并取消待定授权。新HELLO、链路断开、桥停止或15秒请求静默清理会话与授权，旧token不能推进新连接。普通心跳每3秒，授权等待每秒，宿主10秒无应答判为断开。发送成功不代表授权成功；授权等待期间插件阻止其它UI文件操作，可发送SET_DEV(0)取消。

状态文件/临时文件及其祖先目录不可通过文件操作改写或重命名。关闭权限会清理系统区域的未提交上传。手环显示关闭权限、用户拒绝、超时等结果均以实际协议状态为准。策略“开启”表示允许插件请求，不会自动打开开发者模式。

## 停用事件（插件0.2.3）

`flags=2, opcode=0xC2, status=0, req=0`；payload与PING的8B状态布局相同。外层ID低16位=0，高16位为关闭前会话前缀。connected=0时插件立即结束连接，清除开发者模式与待定授权并取消文件操作。命名空间/CRC/连接前缀校验仍强制执行，旧会话事件不影响新会话。

手环开关的原生门禁在配置写盘之前关闭，不等待维护timer；已排队的旧请求和文件应答均作废。仅允许BT owner尝试发送这条停用通知（最多3次，失败仍以心跳超时兜底）。重新开启仅接受新HELLO，不恢复旧会话和授权。

## NSH终端（0.3.0）

HELLO capabilities bit `0x10` 表示独立NSH后端可用。插件由实际载波获得插件权限，手环中需单独允许mask `0x20`，默认关闭；与Android端NSH权限独立。

- `0x50 OPEN`：空请求；返回 `lease:u32LE + state:u8 + error:i32LE`。
- `0x51 WRITE`：`lease + bytes`，输入不超过协商chunk，成功返回空。
- `0x52 READ`：`lease + capacity:u16LE`，返回9字节状态头及输出。
- `0x53 CLOSE`：`lease`，成功返回空。

state：0空闲／1启动中／2运行中／3已退出／4失败／5停止中。所有操作绑定当前连接prefix与lease。一次只有一个NSH请求在途；写入失败或超时不重发命令，10秒无应答结束连接。启动时不与文件传输／测速同时发起。停止期间等待当前应答后关闭已打开的lease；服务停用、设备切换、断线清空本地状态。每次读取/输入仍由模块执行插件NSH权限门禁。

参考：[v4概览](https://abox.run/docs/plugin-development/v4)、[UI](https://abox.run/docs/plugin-development/v2/host-api/ui-v3)、[Timer](https://abox.run/docs/plugin-development/v2/host-api/timer)、[Transport](https://abox.run/docs/plugin-development/v2/host-api/transport)。v4继续采用项目已有`psys_host_v4`异步WIT接口。
