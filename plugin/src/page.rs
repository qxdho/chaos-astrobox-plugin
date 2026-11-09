use crate::astrobox::psys_host_v4::ui;
use crate::protocol;
use crate::state::{self, TransferKind, PAGE_SIZE, State};
use crate::view::View;

const BG: &str = "#0b1220";
const CARD: &str = "#111c2f";
const CARD2: &str = "#16243a";
const TEXT: &str = "#e8eefb";
const MUTED: &str = "#93a4c3";
const ACCENT: &str = "#5aa9ff";
const DANGER: &str = "#ff8b8b";
const OK: &str = "#86e3a6";

fn title(text: &str, size: u32) -> View {
    View::text(text).font_size(size).color(TEXT)
}

fn muted(text: impl Into<String>) -> View {
    View::text(text).font_size(13).color(MUTED)
}

fn card() -> View {
    View::new(ui::ElementType::Card, None)
        .column()
        .padding(12)
        .gap(8)
        .width_full()
        .radius(12)
        .bg(CARD)
}

fn row() -> View {
    View::new(ui::ElementType::Div, None)
        .row()
        .gap(8)
        .align_center()
        .width_full()
}

fn badge(text: &str, color: &str) -> View {
    View::new(ui::ElementType::Badge, Some(text))
        .font_size(11)
        .color(color)
}

fn button(label: &str, event: &str, primary: bool) -> View {
    let mut button = View::button(label, event);
    if primary {
        button = button.prop("variant", "solid");
    }
    button
}

fn device_select(state: &State) -> Option<View> {
    if state.devices.len() < 2 {
        return None;
    }
    let mut select = View::new(ui::ElementType::Select, None)
        .width_full()
        .prop("default-value", &state.addr)
        .on(crate::view::On::Change, "input:device");
    for device in &state.devices {
        select = select.child(
            View::new(ui::ElementType::Option, Some(&device.name)).prop("value", &device.addr),
        );
    }
    Some(select)
}

pub fn build(state: &State) -> View {
    let mut root = View::new(ui::ElementType::Div, None)
        .column()
        .padding(16)
        .gap(12)
        .width_full()
        .bg(BG);

    let busy = state.busy || state.transfer.is_some();
    let mut header = row()
        .child(title("手环文件管理器", 22).grow())
        .child(badge(
            if state.connected { "已连接" } else { "未连接" },
            if state.connected { OK } else { MUTED },
        ));
    if state.connected {
        header = header.child(
            button("重新连接", "action:reconnect", false).disabled_if(busy),
        );
    }
    header = header.child(button("刷新设备", "action:refresh-devices", false));
    if state.connected && state.dev_mode {
        header = header.child(badge("开发者模式", DANGER));
    }
    root = root.child(header);

    if let Some(select) = device_select(state) {
        root = root.child(card().child(muted("设备")).child(select));
    }

    if state.devices.is_empty() {
        root = root.child(
            card()
                .child(title("没有已连接的手环", 17))
                .child(muted("请先在 AstroBox 中连接手环，然后点“刷新设备”。")),
        );
        root = root.child(status_line(state));
        return root;
    }

    if !state.connected {
        root = root.child(
            card()
                .child(title("需要 Chaos 文件桥", 17))
                .child(muted(
                    "本插件通过手环上的 Chaos 原生文件桥读写文件。请确认 Chaos 已安装，文件桥已启动，然后重试连接。",
                ))
                .child(
                    row()
                        .child(button("连接模块", "action:connect", true))
                        .child(button("刷新设备", "action:refresh-devices", false)),
                ),
        );
        root = root.child(status_line(state));
        return root;
    }

    root = root.child(path_bar(state));
    root = root.child(toolbar(state));

    if let Some(transfer) = transfer_card(state) {
        root = root.child(transfer);
    }
    if let Some(detail) = detail_card(state) {
        root = root.child(detail);
    }
    if let Some(preview) = preview_card(state) {
        root = root.child(preview);
    }

    root = root.child(list_card(state));
    root = root.child(status_line(state));
    root
}

fn path_bar(state: &State) -> View {
    card()
        .child(
            row()
                .child(button("上级", "action:up", false).disabled_if(state.path == "/"))
                .child(
                    View::text(state.path.clone())
                        .font_size(15)
                        .color(TEXT)
                        .grow()
                        .prop("class", "truncate"),
                )
                .child(button("刷新", "action:refresh", false)),
        )
        .child(muted(if state.dev_mode {
            "可写：所有路径（/dev、/proc、/sys、/data/canopus 除外）"
        } else {
            "可写：/data（/data/canopus 除外）；切到开发者模式可开放其他路径"
        }))
}

fn toolbar(state: &State) -> View {
    let busy = state.busy || state.transfer.is_some();
    let mut bar = row()
        .child(button("上传文件", "action:upload", true).disabled_if(busy))
        .child(button("新建文件夹", "action:mkdir", false).disabled_if(busy))
        .child(button(
            if state.dev_mode {
                "关闭开发者模式"
            } else {
                "开发者模式"
            },
            "action:dev",
            false,
        ))
        .child(button(if state.dev_mode { "只写 /data" } else { "全路径" }, "action:noop", false).disabled());
    if state.transfer.is_some() {
        bar = bar.child(button("取消传输", "action:cancel", false));
    }
    bar
}

fn transfer_card(state: &State) -> Option<View> {
    let transfer = state.transfer.as_ref()?;
    let percent = if transfer.total == 0 {
        100
    } else {
        (transfer.done as u64 * 100 / transfer.total as u64).min(100)
    };
    let (title_text, detail) = match transfer.kind {
        TransferKind::Upload => (
            "上传中",
            format!("{}  →  {}", transfer.name, transfer.target),
        ),
        TransferKind::Download => (
            "下载中",
            format!("{}  →  本地", transfer.name),
        ),
    };
    Some(
        card()
            .child(
                row()
                    .child(title(title_text, 15).grow())
                    .child(badge(&format!("{percent}%"), ACCENT)),
            )
            .child(muted(detail))
            .child(
                View::new(ui::ElementType::Progress, None)
                    .prop("value", &percent.to_string())
                    .prop("max", "100")
                    .width_full(),
            )
            .child(muted(state::progress_text(transfer.done, transfer.total)))
            .child(button("取消传输", "action:cancel", false)),
    )
}

fn detail_card(state: &State) -> Option<View> {
    let detail = state.detail.as_ref()?;
    let mut card = card()
        .child(
            row()
                .child(title(&detail.name, 16).grow())
                .child(badge(if detail.is_dir { "目录" } else { "文件" }, MUTED)),
        )
        .child(muted(format!("路径：{}", detail.path)))
        .child(muted(format!(
            "大小：{}    修改时间：{}",
            protocol::format_size(detail.size),
            protocol::format_time(detail.mtime)
        )));
    let mut primary = row();
    if !detail.is_dir {
        primary = primary
            .child(button("下载", "action:detail.download", true))
            .child(button("查看/编辑", "action:detail.preview", false));
    }
    primary = primary.child(button("关闭", "action:detail.close", false));
    let mut secondary = row()
        .child(button("重命名", "action:detail.rename", false))
        .child(button("删除", "action:detail.delete", false));
    if !detail.is_dir {
        secondary = secondary.child(button("复制", "action:detail.copy", false));
    }
    card = card.child(primary).child(secondary);
    Some(card)
}

fn preview_card(state: &State) -> Option<View> {
    let preview = state.preview.as_ref()?;
    let mut card = card()
        .child(
            row()
                .child(title(&preview.name, 16).grow())
                .child(badge(&protocol::format_size(preview.size), MUTED)),
        )
        .child(muted(format!("路径：{}", preview.path)));
    if preview.loading {
        card = card.child(muted("正在读取文件…"));
        return Some(card);
    }
    if let Some(error) = &preview.error {
        card = card.child(
            View::text(error.clone())
                .font_size(13)
                .color(DANGER),
        );
        return Some(card.child(button("关闭预览", "action:preview.close", false)));
    }
    if preview.editing {
        card = card
            .child(
                View::new(ui::ElementType::Textarea, Some(&preview.text))
                    .font_size(13)
                    .height(280)
                    .width_full()
                    .bg(CARD2)
                    .radius(8)
                    .padding(8)
                    .on(crate::view::On::Change, "input:preview"),
            )
            .child(
                row()
                    .child(button("保存", "action:preview.save", true))
                    .child(button("取消编辑", "action:preview.cancel", false))
                    .child(button("关闭", "action:preview.close", false)),
            );
    } else {
        card = card
            .child(
                View::text(if preview.text.is_empty() {
                    "（空文件）".to_string()
                } else {
                    preview.text.clone()
                })
                .font_size(12)
                .color(TEXT)
                .width_full(),
            )
            .child(
                row()
                    .child(button("编辑", "action:preview.edit", true))
                    .child(button("关闭", "action:preview.close", false)),
            );
    }
    Some(card)
}

fn list_card(state: &State) -> View {
    let mut card = card();
    if state.loading {
        card = card.child(muted("正在读取目录…"));
        return card;
    }
    if state.entries.is_empty() {
        card = card.child(muted("目录为空"));
        return card;
    }
    let start = state.page * PAGE_SIZE;
    let end = (start + PAGE_SIZE).min(state.entries.len());
    let mut list = View::new(ui::ElementType::Div, None)
        .column()
        .gap(6)
        .width_full();
    for (index, entry) in state.entries.iter().enumerate().skip(start).take(end - start) {
        let event = if entry.is_dir {
            format!("action:enter.{index}")
        } else {
            format!("action:file.{index}")
        };
        list = list.child(
            View::new(ui::ElementType::Div, None)
                .row()
                .align_center()
                .gap(10)
                .padding(12)
                .radius(8)
                .bg(CARD2)
                .on(crate::view::On::Click, &event)
                .child(
                    View::text(if entry.is_dir { "目录" } else { "文件" })
                        .font_size(11)
                        .color(if entry.is_dir { ACCENT } else { MUTED })
                        .width(48),
                )
                .child(
                    View::text(entry.name.clone())
                        .font_size(14)
                        .color(TEXT)
                        .grow(),
                )
                .child(
                    View::text(protocol::format_size(entry.size))
                        .font_size(11)
                        .color(MUTED),
                ),
        );
    }
    card = card.child(list);
    if state.total > PAGE_SIZE as u32 {
        let pages = ((state.total as usize + PAGE_SIZE - 1) / PAGE_SIZE).max(1);
        card = card.child(
            row()
                .child(
                    button("上一页", "action:page.prev", false)
                        .disabled_if(state.page == 0),
                )
                .child(
                    muted(format!(
                        "第 {} / {} 页 · 共 {} 项",
                        state.page + 1,
                        pages,
                        state.total
                    ))
                    .grow(),
                )
                .child(
                    button("下一页", "action:page.next", false)
                        .disabled_if(state.page + 1 >= pages),
                ),
        );
    } else {
        card = card.child(muted(format!("共 {} 项", state.total)));
    }
    card
}

fn status_line(state: &State) -> View {
    View::text(state.status.clone())
        .font_size(12)
        .color(if state.status_error { DANGER } else { MUTED })
        .width_full()
}
