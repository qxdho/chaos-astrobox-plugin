# band_files 协议 v1

CanopusConnect 二进制消息，单条不超过 480 字节。整数均为小端。

```text
请求： [op:u8][req:u16][payload]
应答： [op|0x80][req:u16][status:u8][payload]
事件： [0xC0+n][payload]
```

路径编码：`[len:u16][UTF-8 bytes]`，不以 NUL 结尾；模块侧限制 255 字节并做词法规整（`.`、`..`、`//`）。

## 状态码

| 值 | 含义 |
|---|---|
| 0 | Ok |
| 1 | BadRequest |
| 2 | NotFound |
| 3 | Access |
| 4 | Exist |
| 5 | NotDir |
| 6 | IsDir |
| 7 | NoSpace |
| 8 | Busy / NotEmpty |
| 9 | Io |
| 10 | Crc |
| 11 | Sequence |
| 12 | Unsupported |
| 13 | TooLarge |

## 请求

### `0x01 HELLO`

无 payload。

应答 payload：

```text
u8  protocol_version (=1)
u16 max_data (=448)
u8  flags (bit0 dev_mode_supported, bit1 rmdir_supported)
u8  dev_mode
u16 max_path (=255)
u32 reserved
```

### `0x02 LIST`

```text
path
u32 offset
u8  limit (module caps at 16)
```

应答：

```text
u16 total
u16 offset
u16 count
count × {
  u8  name_len
  u8  name[name_len]
  u8  is_dir
  u32 size
  u32 mtime
}
```

模块每次打开目录遍历，跳过 `offset`，最多返回 `limit` 项或填满 480 字节。插件按 `offset + count < total` 继续翻页。

### `0x03 STAT`

```text
path
```

应答：

```text
u8  is_dir
u32 size
u32 mtime
```

### `0x04 MKDIR` / `0x05 REMOVE` / `0x06 RMDIR`

```text
path
```

`RMDIR` 仅删除空目录，需要 FS 服务版本 ≥2。

### `0x08 COPY`

```text
from_path
to_path
u8 overwrite
```

模块内部流式复制（512 字节块），写到目标同目录临时文件，成功后 rename；目录返回 `IsDir`。

### `0x07 RENAME`

```text
from_path
to_path
```

### `0x10 OPEN_READ`

```text
path
```

应答：

```text
u32 size
```

模块同一时间只保留一个读句柄；新的 `OPEN_READ` 会关闭旧的。

### `0x11 READ`

```text
u32 offset
u16 length (≤448)
```

应答 payload 为原始字节，长度可小于请求长度（文件尾）。

### `0x12 CLOSE_READ`

无 payload。

### `0x20 OPEN_WRITE`

```text
path
u32 expected_size
u8  overwrite
```

模块在目标同目录创建临时文件 `.bftmp`（已存在则先删），后续写入该临时文件。`overwrite=0` 且目标存在返回 `Exist`。

### `0x21 WRITE`

```text
u32 offset
bytes data
```

要求 `offset == 已写入字节数`，否则返回 `Sequence`。模块累计 CRC32；每 8 KiB 或写满时发送事件：

```text
[0xC1][u32 written][u32 crc_running]
```

### `0x22 COMMIT`

```text
u32 crc32_final
```

`crc32_final` 为 `crc_running ^ 0xFFFFFFFF`（标准 CRC32）。模块核对大小与 CRC，`sync`、关闭、`rename(temp, target)`。若目标存在且 `overwrite=1`，rename 返回 `Exist` 时先删除目标再重试。

### `0x23 ABORT`

关闭并删除临时文件。

### `0x30 PING`

应答 1 字节 `0x01`。

### `0x40 SET_DEV`

```text
u8 enabled
```

持久化到模块 KV（键 `dev_mode`）。成功应答无 payload。

## 路径策略

模块在解析路径后、调用 `ctx->fs` 前检查：

- 永远拒绝：`/data/canopus`、`/canopus`、`/dev`、`/proc`、`/sys`（含子路径，大小写不敏感）。
- 默认只允许写 `/data` 及其子路径。
- 开发者模式允许写其它绝对路径，但上述受保护路径仍拒绝。
- 所有路径必须是绝对路径；`..` 不得越过根。

## 限制

- 单文件大小用 u32 表示；超过 4 GiB 不支持。
- 目录项名称受固件 `canopus_fs_dirent.name[64]` 限制（最多 63 字节）。
- 模块同时只有一个读句柄和一个写句柄。
