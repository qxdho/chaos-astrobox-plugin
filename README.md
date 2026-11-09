# 手环文件管理器（AstroBox 插件 + Chaos 原生文件桥）

> 当前推荐形态：插件连接 Chaos 内置的 CanopusConnect 兼容桥，不再需要 Canopus 模块管理器。
> 下文 Canopus 安装步骤仅用于旧版兼容。

在 AstroBox 里浏览、上传、下载和管理 **小米手环 10 Pro（3.101.043）** 的文件。

```text
AstroBox 插件（UI / 本地文件 / 传输）
        │  CanopusConnect 二进制帧，≤ 480 B
        ▼
Canopus 模块 band_files（ctx->fs + ctx->connect）
        ▼
/data、/resource 等手环文件系统
```

## 交付物

| 文件 | 用途 |
|---|---|
| `dist/band-files.abp` | AstroBox 插件安装包 |
| `dist/band_files-0.1.6.canopus` | **推荐**：AstroBox 直接安装的 Canopus 模块包（tar + manifest + CMI2 回执） |
| `dist/installer/band-files-installer.bin` | 备用/兼容路径：p67 安装表盘，供旧版 AstroBox 或无法直接装 `.canopus` 的场景使用 |

## 安装（10 Pro / 3.101.043）

### 1. 准备 Canopus

手环上需要已安装并激活 **Canopus 模块管理器**，且 Supervisor 要支持 CMI2 回执与 ABI 2.0（当前版本即可）。没有的话，先在 AstroBox 的资源页搜索“Canopus模块管理器”安装，打开对应表盘点 Run 激活。

### 2. 安装 band_files 模块（推荐直接安装）

当前 AstroBox 支持把 `.canopus` 作为“模块”资源直接安装，不需要先装安装器表盘：

1. 在 AstroBox 里连接手环；
2. 进入设备页/资源页的“模块”分类，点右上角加号或队列加号，选择 `band_files-0.1.6.canopus`；
3. 等待 AstroBox 把模块推送到手环；安装完成后 Canopus 管理器会发出通知；
4. 打开 Canopus 管理器 → “模块” → `band_files` → **“启用”**（不是“立即激活”）；
5. 重启手环，重新打开 Canopus 管理器表盘点 Run，让 Supervisor 带着模块启动。

> `.canopus` 里的 manifest、CMI2 回执、ELF 哈希与目标固件绑定都按 AstroBox Core 的解析规则校验通过。安装后模块默认禁用，必须执行第 4–5 步。
>
> **不要点“立即激活”**：`band_files` 是启动时加载的常驻模块。0.1.6 起模块生命周期标记为“重启加载”，管理器不会再提供“立即激活”按钮，只保留“启用 → 重启 → Run”这一条路径。

**备用路径**：如果你的 AstroBox 版本没有“模块”本地安装入口，或者只接受表盘包，再使用 `dist/installer/band-files-installer.bin`。这是 10 Pro 的 p67 安装表盘，打开后会把模块写进 `/data/canopus/inbox` 并提交 CPC2 INSTALL；后续启用/重启步骤相同。

### 卡住时怎么恢复

1. 长按/重启手环；重启后**不要**再点“立即激活”。
2. 打开 Canopus 管理器确认 `band_files` 状态；如果管理器仍然卡，用 AstroBox 的模块页禁用或移除 `band_files`，或进入 Canopus 安全模式。
3. 恢复后只点“启用”，然后重启 + Run。

### 3. 安装 AstroBox 插件

在 AstroBox 的插件页安装 `band-files.abp`，打开“手环文件管理器”：

1. 确认设备已连接；
2. 点“连接模块”；
3. 连接成功后从 `/data` 开始浏览。

如果提示“模块没有响应”，检查模块是否已启用、是否重启并重新 Run 过管理器。

## 功能

- 目录浏览、面包屑、返回上级、刷新、排序、分页
- **重新连接**：已连接状态下一键释放监听并重新 listen/握手，不用退出插件
- 文件详情：大小、修改时间、目录/文件
- **上传**：本地选择文件 → 分块写入手环 → 进度/取消
- **下载**：手环文件 → 本地保存 → 进度/取消
- 新建文件夹、重命名、移动（同文件系统 rename）、复制文件（模块内流式复制）
- 删除文件、删除空目录
- 开发者模式：默认只写 `/data`；开启后可写其他路径
- 所有上传先写同目录临时文件，校验 CRC32 后原子 rename；中断不会破坏原文件

## 路径与安全

| 模式 | 可写范围 |
|---|---|
| 默认 | `/data`，但排除 `/data/canopus` |
| 开发者模式 | 所有绝对路径，但 `/dev`、`/proc`、`/sys`、`/data/canopus`、`/canopus` 始终拒绝 |

- 系统区域默认只读浏览。
- 删除/覆盖/开启开发者模式都有二次确认。
- 非空目录不会被递归删除；需要先清空目录。
- 开发者模式可以改 `/resource`、`/etc` 等系统文件，**可能让手环无法启动**，只在明确知道要做什么时开启。

## 限制

- 仅支持 **小米手环 10 Pro 3.101.043**。`.036` 没有 CanopusConnect，`.139` 同理；其它固件未验证。
- 上传单文件上限 **32 MiB**（AstroBox 的本地选择接口会把文件读进插件内存）。
- CanopusConnect 每条消息 480 字节，传输速度受逐条往返限制；大文件请耐心等待，传输中不要断开蓝牙。
- 不支持递归删除、批量选择和断点续传。
- 文本预览/编辑上限 256 KiB；二进制文件只可下载。
- 模块必须由 Canopus Supervisor 托管；插件不能自动安装模块。

## 从源码构建

依赖：Rust nightly（`wasm32-wasip2`）、仓库内 `toolchain/xpack-arm-none-eabi-gcc-15.2.1-1.1`、Node.js、`zip`、`tar`。

```sh
# 1. 编译手环模块（ELF32 ARM ET_REL）
./module/build.sh

# 2. 生成回执、安装表盘和 .canopus
node scripts/build_installer.js

# 3. 编译插件并打包 .abp
./scripts/build_plugin.sh

# 4. 模块主机侧协议测试
./module/test_host.sh
```

产物在 `dist/`。

## 目录

```text
band-files/
├── module/
│   ├── band_files.c          # Canopus 模块（无 libc 导入）
│   ├── build.sh              # ARM 交叉编译
│   ├── test_host.c           # 主机侧协议/文件系统测试
│   └── test_host.sh
├── plugin/
│   ├── src/                  # AstroBox V4 插件（Rust → wasm32-wasip2）
│   └── wit/                  # AstroBox WIT（从 Lyra 插件拷贝）
├── scripts/
│   ├── build_installer.js    # 回执 + 安装表盘 + .canopus
│   └── build_plugin.sh
├── docs/PROTOCOL.md
├── dist/
└── README.md
```

## 协议摘要

请求 `[op][req:u16][payload]`，应答 `[op|0x80][req:u16][status][payload]`，小端。

主要 opcode：`HELLO(0x01)`、`LIST(0x02)`、`STAT(0x03)`、`MKDIR(0x04)`、`REMOVE(0x05)`、`RMDIR(0x06)`、`RENAME(0x07)`、`COPY(0x08)`、`OPEN_READ(0x10)`、`READ(0x11)`、`CLOSE_READ(0x12)`、`OPEN_WRITE(0x20)`、`WRITE(0x21)`、`COMMIT(0x22)`、`ABORT(0x23)`、`SET_DEV(0x40)`。

完整字段见 [docs/PROTOCOL.md](docs/PROTOCOL.md)。
