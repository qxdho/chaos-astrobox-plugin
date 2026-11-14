use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use base64::Engine;
use std::sync::Mutex;

use crate::astrobox::psys_host_v4::{
    transport, register,
    device,
    dialog::{self, DialogButton, DialogInfo, DialogStyle, DialogType, FilterConfig, PickConfig},
    timer, ui,
};
use crate::exports::astrobox::psys_plugin_v4::{
    event, event::EventType, lifecycle,
};
use crate::protocol::*;
use crate::state::{
    Detail, Download, Entry, Pending, Preview, Upload, MAX_ENTRIES, MAX_UPLOAD, PAGE_SIZE,
};

mod page;
mod protocol;
mod state;
mod view;

wit_bindgen::generate!({
    path: "wit",
    world: "chaos-connect-v4",
    generate_all,
});

static DEVICES_REQUESTED: AtomicBool = AtomicBool::new(false);
static NEXT_REQ: AtomicU16 = AtomicU16::new(1);
static PREFIX: AtomicU32 = AtomicU32::new(0);
static UPLOAD_BUSY: AtomicBool = AtomicBool::new(false);
static RECEIVE_BUSY: AtomicBool = AtomicBool::new(false);
static RECEIVE_QUEUE: Mutex<VecDeque<Vec<u8>>> = Mutex::new(VecDeque::new());
static WATCHDOG_TIMER: Mutex<Option<u64>> = Mutex::new(None);
static UPLOAD_TIMER: Mutex<Option<u64>> = Mutex::new(None);
static HEARTBEAT:Mutex<Option<(u16,Instant)>>=Mutex::new(None);
static LAST_HEARTBEAT:Mutex<Option<Instant>>=Mutex::new(None);

const EVENT_DEVICE: &str = "input:device";

struct ChaosConnectPlugin;

fn next_req() -> u16 {
    NEXT_REQ.fetch_add(1, Ordering::Relaxed)
}

fn rerender() {
    let snapshot = state::snapshot();
    if let Some(root) = &snapshot.root {
        ui::render(root, page::build(&snapshot).into_element());
    }
}

static PROGRESS_RENDER:Mutex<Option<Instant>>=Mutex::new(None);
fn rerender_progress(){
    let draw={let mut last=PROGRESS_RENDER.lock().unwrap_or_else(|e|e.into_inner());
        if last.is_some_and(|t|t.elapsed().as_millis()<100){false}else{*last=Some(Instant::now());true}};
    if draw{rerender();}
}

fn set_pending(pending: Pending, req: u16) {
    state::with_state(|state| {
        state.pending = Some(pending);
        state.pending_req = req;
    });
}

fn take_pending(req: u16) -> Option<Pending> {
    state::with_state(|state| {
        if state.pending_req == req {
            state.pending.take()
        } else {
            None
        }
    })
}

async fn send_req(op: u8, req: u16, payload: &[u8]) -> Result<(), String> {
    if payload.len()>MAX_PAYLOAD{return Err("消息或路径超过协议容量".into());}
    if op==OP_WRITE&&!state::with_state(|s|s.upload.as_ref().is_some_and(|u|u.inflight.contains_key(&req))){return Err("上传任务已取消".into());}
    let addr = state::selected_addr();
    if addr.is_empty() {
        return Err("没有选择设备".to_string());
    }
    let frame = encode_request(op, req, payload);
    let id=PREFIX.load(Ordering::SeqCst) | req as u32;
    transport::send(addr, wrap_packet(&frame,id)).await
}

async fn request(op: u8, payload: &[u8], pending: Pending) -> Result<(), String> {
    let req = next_req();
    set_pending(pending, req);
    if let Err(error) = send_req(op, req, payload).await {
        state::with_state(|state| {
            state.pending = None;
            state.pending_req = 0;
        });
        return Err(error);
    }
    Ok(())
}

async fn refresh_devices() {
    DEVICES_REQUESTED.store(true, Ordering::SeqCst);
    let devices = device::get_connected_device_list()
        .await
        .into_iter()
        .map(|item| state::DeviceInfo {
            addr: item.addr,
            name: item.name,
        })
        .collect::<Vec<_>>();
    let old=state::selected_addr();
    if !old.is_empty()&&!devices.iter().any(|d|d.addr==old){
        if state::with_state(|s|s.upload.is_some()){cancel_upload_with("设备断开，已取消上传。".into()).await;}
        if state::with_state(|s|s.download.is_some()){cancel_download().await;}
        if state::with_state(|s|s.benchmark.is_some()){end_benchmark(Some("设备已断开。".into()));}
        state::clear_connection();
    }
    state::with_state(|state| {
        state.devices = devices;
        if state.devices.is_empty() {
            state.addr.clear();
            state.connected = false;
            state.entries.clear();
            state.total = 0;
            state.detail = None;
        } else if !state.devices.iter().any(|item| item.addr == state.addr) {
            state.addr = state.devices[0].addr.clone();
            state.connected = false;
            state.entries.clear();
            state.total = 0;
            state.detail = None;
        }
        if state.devices.len() == 1 {
            state.addr = state.devices[0].addr.clone();
        }
    });
    rerender();
}

async fn connect_bridge() {
    *HEARTBEAT.lock().unwrap_or_else(|e|e.into_inner())=None;
    *LAST_HEARTBEAT.lock().unwrap_or_else(|e|e.into_inner())=None;
    let addr = state::selected_addr();
    if addr.is_empty() {
        state::set_status("请先选择已连接的手环。", true);
        rerender();
        return;
    }
    state::with_state(|state| {
        state.connected = false;
        state.dev_mode = false;
        state.dev_authorization = None;
        state.busy = true;
        state.status = "正在连接 ChaosConnect…".to_string();
        state.status_error = false;
    });
    rerender();
    let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().subsec_nanos();
    PREFIX.store((nonce.rotate_left(11)&0xffff0000).max(0x10000),Ordering::SeqCst);
    if let Err(error) = register::register_transport_recv(addr.clone(), register::TransportRecvFiler {
        xiaomi_vela_v5_channel_id: 1, xiaomi_vela_v5_protobuf_typeid: PB_TYPE,
    }).await {
        state::with_state(|state| {
            state.busy = false;
            state.status = format!("无法监听设备传输：{error}。请确认 Chaos 文件桥已启动，然后重试。");
            state.status_error = true;
        });
        rerender();
        return;
    }
    let req = next_req();
    set_pending(Pending::Hello, req);
    if let Err(error) = send_req(OP_HELLO, req, &[]).await {
        state::with_state(|state| {
            state.pending = None;
            state.busy = false;
            state.status = format!("握手失败：{error}");
            state.status_error = true;
        });
        rerender();
        return;
    }
    let _ = timer::set_timeout(8_000, "hello-timeout");
}

async fn reconnect_bridge() {
    let addr = state::selected_addr();
    if addr.is_empty() {
        state::set_status("请先选择已连接的手环。", true);
        rerender();
        return;
    }
    let blocked = state::with_state(|state| state.busy || state.transfer.is_some());
    if blocked {
        state::set_status("传输进行中，暂时不能重新连接。请先取消或等待完成。", true);
        rerender();
        return;
    }
    state::with_state(|state| {
        state.connected = false;
        state.busy = true;
        state.status = "正在重新连接文件桥…".to_string();
        state.status_error = false;
    });
    rerender();
    // 新HELLO使用不同的连接前缀，旧会话延迟应答不能推进当前传输。
    state::clear_connection();
    connect_bridge().await;
}

async fn load_dir(path: String) {
    state::with_state(|state| {
        state.path = path;
        state.entries.clear();
        state.total = 0;
        state.page = 0;
        state.loading = true;
        state.detail = None;
        state.preview = None;
    });
    rerender();
    send_list_page(state::current_path(), 0, Vec::new(), 0).await;
}

async fn send_list_page(path: String, offset: u32, acc: Vec<Entry>, total: u32) {
    let mut payload = Vec::new();
    push_str(&mut payload, &path);
    put_u32(&mut payload, offset);
    payload.push(16);
    if let Err(error) = request(
        OP_LIST,
        &payload,
        Pending::List {
            path,
            offset,
            acc,
            total,
        },
    )
    .await
    {
        state::with_state(|state| {
            state.loading = false;
            state.status = format!("读取目录失败：{error}");
            state.status_error = true;
        });
        rerender();
    }
}

async fn request_stat(path: String) {
    let mut payload = Vec::new();
    push_str(&mut payload, &path);
    if let Err(error) = request(OP_STAT, &payload, Pending::Stat { path }).await {
        state::set_status(format!("读取文件信息失败：{error}"), true);
        rerender();
    }
}

async fn start_preview() {
    let Some(detail) = state::with_state(|state| state.detail.clone()) else {
        return;
    };
    if detail.is_dir {
        return;
    }
    state::with_state(|state| {
        state.preview = Some(Preview {
            path: detail.path.clone(),
            name: detail.name.clone(),
            size: 0,
            data: Vec::new(),
            text: String::new(),
            loading: true,
            editing: false,
            error: None,
        });
        state.status = format!("正在读取 {}…", detail.name);
        state.status_error = false;
    });
    rerender();
    let mut payload = Vec::new();
    push_str(&mut payload, &detail.path);
    if let Err(error) = request(
        OP_OPEN_READ,
        &payload,
        Pending::PreviewOpen {
            path: detail.path,
            name: detail.name,
        },
    )
    .await
    {
        state::with_state(|state| {
            if let Some(preview) = state.preview.as_mut() {
                preview.loading = false;
                preview.error = Some(error.clone());
            }
            state.status = format!("预览失败：{error}");
            state.status_error = true;
        });
        rerender();
    }
}

async fn request_preview_read(offset: u32) {
    let (size, name) = state::with_state(|state| {
        state
            .preview
            .as_ref()
            .map(|preview| (preview.size, preview.name.clone()))
            .unwrap_or((0, String::new()))
    });
    let len = state::with_state(|s|s.chunk).min(size.saturating_sub(offset) as usize) as u16;
    if len == 0 {
        finish_preview_read();
        return;
    }
    let mut payload = Vec::new();
    put_u32(&mut payload, offset);
    put_u16(&mut payload, len);
    if let Err(error) = request(OP_READ, &payload, Pending::PreviewRead { offset }).await {
        state::with_state(|state| {
            if let Some(preview) = state.preview.as_mut() {
                preview.loading = false;
                preview.error = Some(error.clone());
            }
            state.status = format!("读取 {} 失败：{error}", name);
            state.status_error = true;
        });
        rerender();
    }
}

fn finish_preview_read() {
    state::with_state(|state| {
        state.pending = None;
        state.pending_req = 0;
        let Some(preview) = state.preview.as_mut() else {
            return;
        };
        preview.loading = false;
        if preview.data.iter().any(|b| *b == 0) {
            preview.error = Some("二进制文件不支持文本预览".to_string());
        } else {
            match String::from_utf8(preview.data.clone()) {
                Ok(text) => preview.text = text,
                Err(_) => preview.error = Some("文件不是 UTF-8 文本，无法预览".to_string()),
            }
        }
    });
    rerender();
}

async fn send_rename(path: String, new_name: String) {
    let new_path = join(&state::current_path(), &new_name);
    let mut payload = Vec::new();
    push_str(&mut payload, &path);
    push_str(&mut payload, &new_path);
    if payload.len() > MAX_PAYLOAD {
        state::set_status("路径过长，无法完成重命名。", true);
        rerender();
        return;
    }
    if let Err(error) = request(OP_RENAME, &payload, Pending::Action { refresh: true }).await {
        state::set_status(format!("重命名失败：{error}"), true);
        rerender();
    }
}

async fn send_copy(path: String, new_name: String) {
    let target = join(&state::current_path(), &new_name);
    let mut payload = Vec::new();
    push_str(&mut payload, &path);
    push_str(&mut payload, &target);
    payload.push(1);
    if payload.len() > MAX_PAYLOAD {
        state::set_status("路径过长，无法完成复制。", true);
        rerender();
        return;
    }
    if let Err(error) = request(OP_COPY, &payload, Pending::Action { refresh: true }).await {
        state::set_status(format!("复制失败：{error}"), true);
        rerender();
    }
}

async fn send_mkdir(name: String) {
    let path = join(&state::current_path(), &name);
    let mut payload = Vec::new();
    push_str(&mut payload, &path);
    if let Err(error) = request(OP_MKDIR, &payload, Pending::Action { refresh: true }).await {
        state::set_status(format!("新建文件夹失败：{error}"), true);
        rerender();
    }
}

async fn send_delete(path: String, is_dir: bool) {
    let mut payload = Vec::new();
    push_str(&mut payload, &path);
    let op = if is_dir { OP_RMDIR } else { OP_REMOVE };
    if let Err(error) = request(op, &payload, Pending::Action { refresh: true }).await {
        state::set_status(format!("删除失败：{error}"), true);
        rerender();
    }
}

async fn send_set_dev(enabled: bool) {
    if !state::with_state(|s|s.connected&&!s.busy&&s.transfer.is_none()){return;}
    let previous_authorization=state::with_state(|s|s.dev_authorization);
    state::with_state(|s|{
        s.busy=true;
        s.dev_authorization=Some((0,Instant::now()));
        s.status=if enabled{"正在请求手环开启开发者模式…".into()}else{"正在关闭开发者模式…".into()};
        s.status_error=false;
    });
    rerender();
    let req = next_req();
    set_pending(Pending::SetDev { enabled }, req);
    if let Err(error) = send_req(OP_SET_DEV, req, &[if enabled { 1 } else { 0 }]).await {
        state::with_state(|state| {
            state.pending = None;
            state.busy = false;
            state.dev_authorization = None;
            state.status = format!("切换开发者模式失败：{error}");
            state.status_error = true;
            if !enabled&&previous_authorization.is_some(){
                state.busy=true;state.dev_authorization=previous_authorization;
                state.status=format!("取消请求未送达：{error}。请在手环选择拒绝，或等待授权超时。");
            }
        });
        rerender();
    }
}

async fn dialog_input(title: &str, content: &str) -> Option<String> {
    let info = DialogInfo {
        title: title.to_string(),
        content: content.to_string(),
        buttons: vec![
            DialogButton {
                id: "cancel".to_string(),
                primary: false,
                content: "取消".to_string(),
            },
            DialogButton {
                id: "ok".to_string(),
                primary: true,
                content: "确定".to_string(),
            },
        ],
    };
    let result = dialog::show_dialog(DialogType::Input, DialogStyle::System, info).await;
    if result.clicked_btn_id == "ok" {
        let value = result.input_result.trim().to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

async fn dialog_confirm(title: &str, content: &str, ok_label: &str) -> bool {
    let info = DialogInfo {
        title: title.to_string(),
        content: content.to_string(),
        buttons: vec![
            DialogButton {
                id: "cancel".to_string(),
                primary: false,
                content: "取消".to_string(),
            },
            DialogButton {
                id: "ok".to_string(),
                primary: true,
                content: ok_label.to_string(),
            },
        ],
    };
    let result = dialog::show_dialog(DialogType::Alert, DialogStyle::System, info).await;
    result.clicked_btn_id == "ok"
}

fn sanitize_name(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name).trim();
    let mut out = String::new();
    for ch in base.chars() {
        if ch == '/' || ch == '\\' || (ch as u32) < 0x20 {
            out.push('_');
        } else {
            out.push(ch);
        }
        if out.len() >= 200 {
            break;
        }
    }
    let out = out.trim().to_string();
    if out.is_empty() || out == "." || out == ".." {
        "upload.bin".to_string()
    } else {
        out
    }
}

fn timer_payload(payload: &str) -> &str {
    let Some(pos) = payload.find("\"payload\"") else {
        return payload;
    };
    let rest = &payload[pos + 9..];
    let Some(colon) = rest.find(':') else {
        return payload;
    };
    let rest = rest[colon + 1..].trim_start();
    let Some(rest) = rest.strip_prefix('"') else {
        return payload;
    };
    match rest.find('"') {
        Some(end) => &rest[..end],
        None => payload,
    }
}

fn event_value(payload: &str) -> String {
    let Some(pos) = payload.find("\"value\"") else {
        return String::new();
    };
    let rest = &payload[pos + 7..];
    let Some(colon) = rest.find(':') else {
        return String::new();
    };
    let rest = rest[colon + 1..].trim_start();
    if let Some(rest) = rest.strip_prefix('"') {
        let mut out = String::new();
        let mut escaped = false;
        for ch in rest.chars() {
            if escaped {
                out.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                break;
            } else {
                out.push(ch);
            }
        }
        out
    } else {
        String::new()
    }
}

async fn start_upload_bytes(target: String, name: String, data: Vec<u8>) {
    if data.len() > MAX_UPLOAD {
        state::set_status("文件超过 32 MiB 上限。", true);
        rerender();
        return;
    }
    state::with_state(|state| {
        state.upload = Some(Upload {
            target: target.clone(),
            name: name.clone(),
            data,
            offset: 0,
            crc: 0xFFFF_FFFF,
            overwrite: true,
            started: false,
            timer: None,
            reported: 0,
            inflight: HashMap::new(), clock: None, last_ack: Instant::now(), committing: false,
        });
        state.status = format!("准备保存 {name}…");
        state.status_error = false;
    });
    rerender();
    send_open_write(true).await;
}

async fn start_upload() {
    if state::with_state(|state| state.upload.is_some() || state.download.is_some()) {
        state::set_status("已有传输任务，请先完成或取消。", true);
        rerender();
        return;
    }
    state::set_status("正在选择本地文件…", false);
    rerender();
    let filter = FilterConfig {
        multiple: false,
        extensions: Vec::new(),
        default_directory: String::new(),
        default_file_name: String::new(),
    };
    // AstroBox 的 read:true 在部分 Android 版本只返回文件名、data 为空。
    // 和 Lyra 导入器一样，让宿主先把文件复制进插件沙箱，再用 std::fs 读。
    let copy_dir = "media";
    let picked = dialog::pick_file(
        PickConfig {
            read: false,
            copy_to: Some(copy_dir.to_string()),
        },
        filter,
    )
    .await;
    let picked = match picked {
        Ok(value) => value,
        Err(error) => {
            state::set_status(format!("选择文件失败：{error}"), true);
            rerender();
            return;
        }
    };
    let copied_path = format!("{copy_dir}/{}", picked.name);
    let data = match std::fs::read(&copied_path) {
        Ok(data) => data,
        Err(first_error) => match std::fs::read(&picked.name) {
            Ok(data) => data,
            Err(second_error) => {
                state::set_status(
                    format!(
                        "读取所选文件失败：{first_error} / {second_error}。请确认 AstroBox 有存储权限。"
                    ),
                    true,
                );
                rerender();
                return;
            }
        },
    };
    let _ = std::fs::remove_file(&copied_path);
    if data.is_empty() {
        state::set_status("所选文件是空文件，未上传。", true);
        rerender();
        return;
    }
    if data.len() > MAX_UPLOAD {
        state::set_status("文件超过 32 MiB 上限。请先在本地拆分后再上传。", true);
        rerender();
        return;
    }
    let name = sanitize_name(&picked.name);
    let target = join(&state::current_path(), &name);
    state::with_state(|state| {
        state.upload = Some(Upload {
            target: target.clone(),
            name: name.clone(),
            data,
            offset: 0,
            crc: 0xFFFF_FFFF,
            overwrite: false,
            started: false,
            timer: None,
            reported: 0,
            inflight: HashMap::new(), clock: None, last_ack: Instant::now(), committing: false,
        });
        state.status = format!("准备上传 {name}…");
        state.status_error = false;
    });
    rerender();
    send_open_write(false).await;
}

async fn send_open_write(overwrite: bool) {
    let (target, size) = match state::with_state(|state| {
        state.upload.as_ref().map(|upload| (upload.target.clone(), upload.data.len() as u32))
    }) {
        Some(value) => value,
        None => return,
    };
    state::with_state(|state| {
        if let Some(upload) = state.upload.as_mut() {
            upload.overwrite = overwrite;
        }
    });
    let mut payload = Vec::new();
    push_str(&mut payload, &target);
    put_u32(&mut payload, size);
    payload.push(if overwrite { 1 } else { 0 });
    if let Err(error) = request(OP_OPEN_WRITE, &payload, Pending::UploadOpen).await {
        state::with_state(|state| state.upload = None);
        state::set_status(format!("打开上传目标失败：{error}"), true);
        rerender();
    }
}

const BENCH_BYTES:u32=1024*1024;
static BENCH_BUSY:AtomicBool=AtomicBool::new(false);
static BENCH_TIMER:Mutex<Option<u64>>=Mutex::new(None);
fn end_benchmark(reason:Option<String>){
    if let Some(id)=BENCH_TIMER.lock().unwrap_or_else(|e|e.into_inner()).take(){timer::clear_timer(id);}
    state::with_state(|s|{if let Some(b)=s.benchmark.take(){
        s.speed=speed_text(b.received,b.clock);
        s.status=reason.clone().unwrap_or_else(||format!("已回显校验1 MiB，单方向有效速度：{}",s.speed));
        s.status_error=reason.is_some();s.busy=false;}});rerender();
}
async fn start_benchmark(){
    if state::with_state(|s|s.busy||s.upload.is_some()||s.download.is_some()){return;}
    state::with_state(|s|{s.busy=true;s.benchmark=Some(state::Benchmark{sent:0,received:0,inflight:HashMap::new(),clock:Instant::now(),last_ack:Instant::now()});s.status="正在进行1 MiB链路回显测试（不会写入手环文件）…".into();s.status_error=false;});
    let id=timer::set_interval(100,"benchmark");
    if id==0{end_benchmark(Some("无法启动测速计时器。".into()));return;}
    *BENCH_TIMER.lock().unwrap_or_else(|e|e.into_inner())=Some(id);drive_benchmark().await;
}
async fn drive_benchmark(){
    if BENCH_BUSY.swap(true,Ordering::SeqCst){return;}
    if state::with_state(|s|s.benchmark.as_ref().is_some_and(|b|b.last_ack.elapsed().as_secs()>=15)){
        BENCH_BUSY.store(false,Ordering::SeqCst);end_benchmark(Some("测速等待应答超时，请检查连接。".into()));return;
    }
    for _ in 0..WINDOW{
        let next=state::with_state(|s|{let n=s.chunk;let b=s.benchmark.as_mut()?;if b.sent>=BENCH_BYTES||b.inflight.len()>=WINDOW{return None;}
            let offset=b.sent;let n=n.min((BENCH_BYTES-offset)as usize);let req=next_req();b.inflight.insert(req,(offset,n));b.sent+=n as u32;Some((req,offset,n))});
        let Some((req,offset,n))=next else{break;};
        let data=(0..n).map(|i|((offset as usize+i)*31)as u8).collect::<Vec<_>>();
        if let Err(e)=send_req(0x31,req,&data).await{BENCH_BUSY.store(false,Ordering::SeqCst);end_benchmark(Some(format!("测速发送失败：{e}")));return;}
    }
    state::with_state(|s|{if let Some(b)=&s.benchmark{s.speed=speed_text(b.received,b.clock);}});
    BENCH_BUSY.store(false,Ordering::SeqCst);rerender_progress();
}

fn speed_text(bytes:u32,clock:Instant)->String {
    let rate=bytes as f64/clock.elapsed().as_secs_f64().max(0.001);
    format!("{:.1} KB/s · {:.0} kb/s{}",rate/1000.0,rate*8.0/1000.0,if rate>=25000.0{" · 达到200 kb/s"}else{" · 未达到200 kb/s"})
}
async fn drive_upload() {
    if UPLOAD_BUSY.swap(true, Ordering::SeqCst){return;}
    let expired=state::with_state(|s|s.upload.as_ref().is_some_and(|u|u.started&&u.last_ack.elapsed().as_secs()>=15));
    if expired{UPLOAD_BUSY.store(false,Ordering::SeqCst);cancel_upload_with("上传等待应答超时，临时文件已取消。".into()).await;return;}
    for _ in 0..WINDOW {
        let next=state::with_state(|s|{
            let chunk=s.chunk;let u=s.upload.as_mut()?;
            if !u.started||u.committing||u.offset>=u.data.len()||u.inflight.len()>=WINDOW{return None;}
            let offset=u.offset;let end=(offset+chunk).min(u.data.len());let data=u.data[offset..end].to_vec();
            let req=next_req();u.crc=crc32(&data,u.crc);u.offset=end;u.inflight.insert(req,(offset as u32,data.len()));
            Some((req,offset as u32,data))
        });
        let Some((req,offset,data))=next else{break;};
        let mut payload=Vec::with_capacity(4+data.len());put_u32(&mut payload,offset);payload.extend_from_slice(&data);
        if let Err(e)=send_req(OP_WRITE,req,&payload).await{
            UPLOAD_BUSY.store(false,Ordering::SeqCst);cancel_upload_with(format!("上传中断：{e}")).await;return;
        }
    }
    let commit=state::with_state(|s|{
        let u=s.upload.as_mut()?;
        if !u.ready_commit(){return None;}
        u.committing=true;Some(u.crc^0xffffffff)
    });
    if let Some(crc)=commit{
        if let Some(id)=UPLOAD_TIMER.lock().unwrap_or_else(|e|e.into_inner()).take(){timer::clear_timer(id);}
        let mut payload=Vec::new();put_u32(&mut payload,crc);
        if let Err(e)=request(OP_COMMIT,&payload,Pending::Commit).await{state::with_state(|s|s.upload=None);state::set_status(format!("提交上传失败：{e}"),true);}
    }
    UPLOAD_BUSY.store(false,Ordering::SeqCst);
    state::with_state(|s|{if let Some(u)=&s.upload{if let Some(clock)=u.clock{s.speed=speed_text(u.reported,clock);}}});
    rerender_progress();
}

async fn cancel_upload_with(message: String) {
    if let Some(timer_id) = UPLOAD_TIMER.lock().unwrap_or_else(|e| e.into_inner()).take() {
        timer::clear_timer(timer_id);
    }
    if state::with_state(|state| state.upload.is_some()) {
        let req = next_req();
        let _ = send_req(OP_ABORT, req, &[]).await;
    }
    state::with_state(|state| {
        state.pending = None;
        state.pending_req = 0;
        state.upload = None;
    });
    state::set_status(message, true);
    rerender();
}

async fn cancel_upload() {
    cancel_upload_with("已取消上传。".to_string()).await;
}

async fn start_download() {
    if state::with_state(|state| state.upload.is_some() || state.download.is_some()) {
        state::set_status("已有传输任务，请先完成或取消。", true);
        rerender();
        return;
    }
    let Some(detail) = state::with_state(|state| state.detail.clone()) else {
        return;
    };
    if detail.is_dir {
        return;
    }
    state::set_status("正在选择本地保存位置…", false);
    rerender();
    let extension = detail
        .name
        .rsplit_once('.')
        .map(|(_, ext)| vec![ext.to_ascii_lowercase()])
        .unwrap_or_default();
    let filter = FilterConfig {
        multiple: false,
        extensions: extension,
        default_directory: String::new(),
        default_file_name: detail.name.clone(),
    };
    let session = match dialog::save_file_start(filter).await {
        Ok(session) => session,
        Err(error) => {
            state::set_status(format!("无法创建本地文件：{error}。请确认 AstroBox 有存储权限，并在系统保存框里选择位置。"), true);
            rerender();
            return;
        }
    };
    state::with_state(|state| {
        state.download = Some(Download {
            session: session.session_id,
            path: detail.path.clone(),
            name: if session.name.is_empty() {
                detail.name.clone()
            } else {
                session.name.clone()
            },
            size: 0,
            next_issue: 0,
            next_write: 0,
            received: BTreeMap::new(),
            inflight: HashMap::new(),
            written: 0,
            closing: false,
            clock: Instant::now(), last_ack: Instant::now(),
        });
        state.status = format!("准备下载 {}…", detail.name);
        state.status_error = false;
    });
    rerender();
    let mut payload = Vec::new();
    push_str(&mut payload, &detail.path);
    if let Err(error) = request(
        OP_OPEN_READ,
        &payload,
        Pending::DownloadOpen {
            session: session.session_id,
            path: detail.path,
            name: detail.name,
        },
    )
    .await
    {
        let _ = dialog::save_file_abort(session.session_id).await;
        state::with_state(|state| state.download = None);
        state::set_status(format!("打开手环文件失败：{error}"), true);
        rerender();
    }
}

async fn cancel_download() {
    let info = state::with_state(|state| {
        state.download.as_mut().map(|download| (download.session, download.closing))
    });
    if let Some((session, closing)) = info {
        if !closing {
            let _ = dialog::save_file_abort(session).await;
        }
        let req = next_req();
        let _ = send_req(OP_CLOSE_READ, req, &[]).await;
    }
    state::with_state(|state| {
        state.pending = None;
        state.pending_req = 0;
        state.download = None;
    });
    state::set_status("已取消下载。", false);
    rerender();
}

async fn fill_read_window() {
    loop {
        let next = state::with_state(|state| {
            let Some(download) = state.download.as_mut() else {
                return None;
            };
            if download.closing || download.next_issue >= download.size {
                return None;
            }
            if download.inflight.len() >= WINDOW {
                return None;
            }
            let offset = download.next_issue;
            let len = state.chunk.min((download.size - offset) as usize) as u16;
            download.next_issue += len as u32;
            let req = next_req();
            download.inflight.insert(req, offset);
            Some((req, offset, len))
        });
        let Some((req, offset, len)) = next else {
            return;
        };
        let mut payload = Vec::new();
        put_u32(&mut payload, offset);
        put_u16(&mut payload, len);
        if let Err(error) = send_req(OP_READ, req, &payload).await {
            state::set_status(format!("读取失败：{error}"), true);
            cancel_download().await;
            return;
        }
    }
}

async fn finish_download() {
    let info = state::with_state(|state| {
        state.download.as_ref().map(|download| {
            (
                download.session,
                download.name.clone(),
                download.size,
            )
        })
    });
    if let Some((session, name, size)) = info {
        let req = next_req();
        let _ = send_req(OP_CLOSE_READ, req, &[]).await;
        match dialog::save_file_finish(session).await {
            Ok(()) => state::set_status(
                format!("已下载 {name}（{}）", format_size(size)),
                false,
            ),
            Err(error) => state::set_status(format!("保存本地文件失败：{error}"), true),
        }
    }
    state::with_state(|s|{if let Some(d)=&s.download{s.speed=speed_text(d.written,d.clock);}s.download=None;});
    rerender();
}

async fn on_read_response(status: u8, payload: &[u8], offset: u32) {
    if status != ST_OK {
        state::set_status(format!("下载中断：{}", status_text(status)), true);
        cancel_download().await;
        return;
    }
    let valid=state::with_state(|s|s.download.as_ref().is_some_and(|d|offset<d.size&&payload.len()==s.chunk.min((d.size-offset)as usize)));
    if !valid{state::set_status("下载块长度不符合协商结果，已取消。",true);cancel_download().await;return;}
    let chunk = payload.to_vec();
    let (session, chunks, done) = state::with_state(|state| {
        let Some(download) = state.download.as_mut() else {
            return (0, Vec::new(), true);
        };
        download.last_ack=Instant::now();
        download.received.insert(offset, chunk);
        let mut chunks = Vec::new();
        while let Some(data) = download.received.remove(&download.next_write) {
            download.next_write += data.len() as u32;
            download.written = download.next_write;
            chunks.push(data);
        }
        (download.session, chunks, download.next_write >= download.size)
    });
    if session == 0 {
        return;
    }
    for chunk in chunks {
        if let Err(error) = dialog::save_file_write_chunk(session, chunk).await {
            state::set_status(format!("写入本地文件失败：{error}"), true);
            cancel_download().await;
            return;
        }
    }
    if done {
        finish_download().await;
        return;
    }
    fill_read_window().await;
    state::with_state(|s|{if let Some(d)=&s.download{s.speed=speed_text(d.written,d.clock);}});
    rerender_progress();
}

async fn handle_ui_event(event_id: &str, payload: &str) {
    if state::with_state(|s|s.dev_authorization.is_some())&&event_id!="action:cancel-authorization"{
        state::set_status("请先在手环完成授权，或取消本次请求。",false);rerender();return;
    }
    if event_id=="action:cancel-authorization"{
        // A disabling request also invalidates the device's pending authorization token.
        state::with_state(|s|s.busy=false);
        send_set_dev(false).await;return;
    }
    if event_id == "input:preview" {
        let value = event_value(payload);
        state::with_state(|state| {
            if let Some(preview) = state.preview.as_mut() {
                preview.text = value;
            }
        });
        return;
    }
    if event_id == EVENT_DEVICE {
        if state::with_state(|s|s.busy||s.upload.is_some()||s.download.is_some()){
            state::set_status("请先取消或完成当前传输，再切换设备。",true);rerender();return;
        }
        let value = event_value(payload);
        if !value.is_empty() {
            state::with_state(|state| {
                state.addr = value;
                state.connected = false;
                state.entries.clear();
                state.total = 0;
                state.detail = None;
            });
            connect_bridge().await;
        }
        return;
    }
    if event_id == "action:refresh-devices" {
        refresh_devices().await;
        return;
    }
    if event_id == "action:connect" {
        connect_bridge().await;
        return;
    }
    if event_id == "action:reconnect" {
        reconnect_bridge().await;
        return;
    }
    if event_id == "action:refresh" {
        load_dir(state::current_path()).await;
        return;
    }
    if event_id == "action:up" {
        let path = crate::protocol::parent(&state::current_path());
        load_dir(path).await;
        return;
    }
    if event_id=="action:benchmark"{start_benchmark().await;return;}
    if event_id=="action:cancel-benchmark"{end_benchmark(Some("已取消测速。".into()));return;}
    if event_id == "action:upload" {
        start_upload().await;
        return;
    }
    if event_id == "action:cancel" {
        if state::with_state(|s| s.upload.is_some()) {
            cancel_upload().await;
        } else {
            cancel_download().await;
        }
        return;
    }
    if event_id == "action:mkdir" {
        if let Some(name) = dialog_input("新建文件夹", "输入文件夹名称").await {
            let name = sanitize_name(&name);
            send_mkdir(name).await;
        }
        return;
    }
    if event_id == "action:dev" {
        let enabled = state::with_state(|s| s.dev_mode);
        if enabled {
            if dialog_confirm(
                "关闭开发者模式",
                "关闭后将只能写 /data。",
                "关闭",
            )
            .await
            {
                send_set_dev(false).await;
            }
        } else if dialog_confirm(
            "开启开发者模式",
            "开启后可写入 /data 以外的路径（/dev、/proc、/sys 仍被拒绝）。这会显著增加误删系统文件的风险，只在明确知道要做什么时使用。",
            "开启",
        )
        .await
        {
            send_set_dev(true).await;
        }
        return;
    }
    if event_id == "action:page.prev" {
        state::with_state(|state| state.page = state.page.saturating_sub(1));
        rerender();
        return;
    }
    if event_id == "action:page.next" {
        state::with_state(|state| {
            let pages = ((state.total as usize + PAGE_SIZE - 1) / PAGE_SIZE).max(1);
            if state.page + 1 < pages {
                state.page += 1;
            }
        });
        rerender();
        return;
    }
    if event_id == "action:detail.close" {
        state::with_state(|state| {
            state.detail = None;
            state.preview = None;
        });
        rerender();
        return;
    }
    if event_id == "action:detail.download" {
        start_download().await;
        return;
    }
    if event_id == "action:detail.preview" {
        start_preview().await;
        return;
    }
    if event_id == "action:preview.edit" {
        state::with_state(|state| {
            if let Some(preview) = state.preview.as_mut() {
                preview.editing = true;
            }
        });
        rerender();
        return;
    }
    if event_id == "action:preview.cancel" {
        state::with_state(|state| {
            if let Some(preview) = state.preview.as_mut() {
                preview.editing = false;
            }
        });
        rerender();
        return;
    }
    if event_id == "action:preview.close" {
        state::with_state(|state| state.preview = None);
        rerender();
        return;
    }
    if event_id == "action:preview.save" {
        let info = state::with_state(|state| {
            state.preview.as_ref().map(|preview| {
                (preview.path.clone(), preview.name.clone(), preview.text.clone().into_bytes())
            })
        });
        if let Some((path, name, data)) = info {
            start_upload_bytes(path, name, data).await;
        }
        return;
    }
    if event_id == "action:detail.copy" {
        let Some(detail) = state::with_state(|state| state.detail.clone()) else {
            return;
        };
        if detail.is_dir {
            state::set_status("目录复制尚未支持。", true);
            rerender();
            return;
        }
        if let Some(name) = dialog_input("复制文件", &format!("复制 {} 到当前目录，输入新名称", detail.name)).await {
            if name.contains('/') || name == "." || name == ".." {
                state::set_status("名称不能包含 /，也不能是 . 或 ..", true);
                rerender();
                return;
            }
            let target = join(&state::current_path(), &name);
            if dialog_confirm("确认复制", &format!("将复制到：\n{target}\n\n同名文件会被覆盖。"), "复制").await {
                send_copy(detail.path, name).await;
            }
        }
        return;
    }
    if event_id == "action:detail.rename" {
        let Some(detail) = state::with_state(|state| state.detail.clone()) else {
            return;
        };
        if let Some(name) = dialog_input("重命名", &format!("当前名称：{}\n输入新名称", detail.name)).await {
            if name.contains('/') || name == "." || name == ".." {
                state::set_status("名称不能包含 /，也不能是 . 或 ..", true);
                rerender();
                return;
            }
            send_rename(detail.path, name).await;
        }
        return;
    }
    if event_id == "action:detail.delete" {
        let Some(detail) = state::with_state(|state| state.detail.clone()) else {
            return;
        };
        let label = if detail.is_dir { "删除目录" } else { "删除文件" };
        if dialog_confirm(
            label,
            &format!("确定删除？\n{}\n\n此操作不可撤销。空目录可直接删除；非空目录会失败。", detail.path),
            "删除",
        )
        .await
        {
            send_delete(detail.path, detail.is_dir).await;
        }
        return;
    }
    if let Some(index) = event_id.strip_prefix("action:enter.") {
        if let Ok(index) = index.parse::<usize>() {
            let entry = state::with_state(|state| state.entries.get(index).cloned());
            if let Some(entry) = entry {
                let path = join(&state::current_path(), &entry.name);
                if entry.is_dir {
                    load_dir(path).await;
                } else {
                    request_stat(path).await;
                }
            }
        }
        return;
    }
    if let Some(index) = event_id.strip_prefix("action:file.") {
        if let Ok(index) = index.parse::<usize>() {
            let entry = state::with_state(|state| state.entries.get(index).cloned());
            if let Some(entry) = entry {
                request_stat(join(&state::current_path(), &entry.name)).await;
            }
        }
        return;
    }
}

async fn handle_hello(response: &Response<'_>) {
    if response.status != ST_OK || response.payload.len() < 11 {
        state::with_state(|state| {
            state.busy = false;
            state.status = format!("ChaosConnect握手失败：{}", status_text(response.status));
            state.status_error = true;
        });
        rerender();
        return;
    }
    let version = response.payload[0];
    let dev = response.payload[4] != 0;
    let chunk=get_u16(response.payload,1).unwrap_or(0) as usize;
    if version!=PROTO_VERSION||chunk==0||chunk>CHUNK||get_u32(response.payload,7)!=Some(WINDOW as u32)||response.payload[3]&4==0{
        state::with_state(|s|{s.busy=false;s.connected=false;});
        state::set_status("ChaosConnect版本或协商块大小不兼容，请同时更新插件和Chaos。",true);rerender();return;
    }
    state::with_state(|state| {
        state.connected = true;
        state.chunk=chunk;
        state.busy = false;
        state.dev_mode = dev;
        state.status = format!("已连接 ChaosConnect v{version} · {chunk} B × {WINDOW}窗口");
        state.status_error = false;
    });
    let has_timer={let mut timer=WATCHDOG_TIMER.lock().unwrap_or_else(|e|e.into_inner());
     if timer.is_none(){let id=timer::set_interval(1000,"transfer-watchdog");if id!=0{*timer=Some(id);}}
     timer.is_some()};
    if !has_timer{connection_lost("无法启动心跳定时器，请重启插件后重新连接。").await;return;}
    heartbeat().await;
    if state::with_state(|s|s.connected){load_dir("/data".to_string()).await;}
}

fn parse_list_entries(payload: &[u8]) -> Option<(u16, u16, u16, Vec<Entry>)> {
    let total = get_u16(payload, 0)?;
    let offset = get_u16(payload, 2)?;
    let count = get_u16(payload, 4)?;
    let mut cursor = 6usize;
    let mut entries = Vec::new();
    for _ in 0..count {
        // Module LIST entries use a one-byte name length, not the u16
        // length used by request paths.
        let name_len = *payload.get(cursor)? as usize;
        cursor += 1;
        let name_bytes = payload.get(cursor..cursor + name_len)?;
        let name = String::from_utf8_lossy(name_bytes).into_owned();
        cursor += name_len;
        let entry_type = *payload.get(cursor)?;
        cursor += 1;
        let size = get_u32(payload, cursor)?;
        cursor += 4;
        let mtime = get_u32(payload, cursor)?;
        cursor += 4;
        entries.push(Entry {
            name,
            is_dir: entry_type != 0,
            size,
            mtime,
        });
        if entries.len() >= MAX_ENTRIES {
            break;
        }
    }
    Some((total, offset, count, entries))
}

async fn handle_list(pending: Pending, response: &Response<'_>) {
    let Pending::List {
        path,
        offset,
        mut acc,
        total,
    } = pending
    else {
        return;
    };
    if response.status != ST_OK {
        state::with_state(|state| {
            state.loading = false;
            state.status = format!("读取目录失败：{}", status_text(response.status));
            state.status_error = true;
        });
        rerender();
        return;
    }
    let Some((reported_total, _reported_offset, count, entries)) =
        parse_list_entries(response.payload)
    else {
        state::with_state(|state| {
            state.loading = false;
            state.status = "文件桥返回了无法解析的目录数据".to_string();
            state.status_error = true;
        });
        rerender();
        return;
    };
    for entry in entries {
        if acc.len() >= MAX_ENTRIES {
            break;
        }
        acc.push(entry);
    }
    let total = total.max(reported_total as u32);
    let next_offset = offset + count as u32;
    let more = count > 0 && next_offset < total && acc.len() < MAX_ENTRIES;
    state::with_state(|state| {
        state.entries = acc.clone();
        state.total = total;
        state.loading = more;
        state.page = state.page.min(
            ((state.total as usize + PAGE_SIZE - 1) / PAGE_SIZE)
                .saturating_sub(1),
        );
    });
    rerender();
    if more {
        send_list_page(path, next_offset, acc, total).await;
    } else {
        state::with_state(|state| {
            state.entries.sort_by(|a, b| {
                b.is_dir
                    .cmp(&a.is_dir)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            });
            state.entries.truncate(MAX_ENTRIES);
            state.total = state.total.min(MAX_ENTRIES as u32);
            state.loading = false;
            state.status = format!("共 {} 项", state.total);
            state.status_error = false;
        });
        rerender();
    }
}

async fn handle_stat(pending: Pending, response: &Response<'_>) {
    let Pending::Stat { path } = pending else {
        return;
    };
    if response.status != ST_OK || response.payload.len() < 9 {
        state::set_status(format!("读取信息失败：{}", status_text(response.status)), true);
        rerender();
        return;
    }
    let is_dir = response.payload[0] != 0;
    let size = get_u32(response.payload, 1).unwrap_or(0);
    let mtime = get_u32(response.payload, 5).unwrap_or(0);
    state::with_state(|state| {
        state.detail = Some(Detail {
            name: crate::protocol::file_name(&path).to_string(),
            path,
            is_dir,
            size,
            mtime,
        });
        state.status = if is_dir {
            "目录".to_string()
        } else {
            "文件".to_string()
        };
        state.status_error = false;
    });
    rerender();
}

async fn handle_action(pending: Pending, response: &Response<'_>) {
    let Pending::Action { refresh } = pending else {
        return;
    };
    let ok = response.status == ST_OK;
    state::set_status(
        if ok {
            "操作成功".to_string()
        } else {
            format!("操作失败：{}", status_text(response.status))
        },
        !ok,
    );
    if ok && refresh {
        state::with_state(|state| state.detail = None);
        load_dir(state::current_path()).await;
    } else {
        rerender();
    }
}

async fn handle_upload_open(response: &Response<'_>) {
    if state::with_state(|state| state.upload.is_none()) {
        return;
    }
    if response.status == ST_OK {
        // 30 ms is below the host timer floor on some AstroBox builds and the
        // callback never fires, which looks like a frozen 0% progress bar.
        let timer_id = timer::set_interval(100, "upload");
        if timer_id == 0 {
            state::with_state(|state| {
                state.upload = None;
                state.status = "无法启动上传计时器，请重启插件后重试。".to_string();
                state.status_error = true;
            });
            rerender();
            return;
        }
        *UPLOAD_TIMER.lock().unwrap_or_else(|e| e.into_inner()) = Some(timer_id);
        state::with_state(|state| {
            if let Some(upload) = state.upload.as_mut() {
                upload.started = true;
                upload.clock=Some(Instant::now());upload.last_ack=Instant::now();
                upload.timer = Some(timer_id);
            }
            state.status = "上传中…".to_string();
            state.status_error = false;
        });
        rerender();
        return;
    }
    if response.status == ST_EXIST {
        let target = state::with_state(|state| state.upload.as_ref().map(|u| u.target.clone()));
        let Some(target) = target else {
            return;
        };
        if dialog_confirm(
            "文件已存在",
            &format!("目标已存在：\n{target}\n\n是否覆盖？"),
            "覆盖",
        )
        .await
        {
            send_open_write(true).await;
        } else {
            state::with_state(|state| state.upload = None);
            state::set_status("已取消上传。", false);
            rerender();
        }
        return;
    }
    state::with_state(|state| {
        state.upload = None;
        state.status = format!("无法创建目标文件：{}", status_text(response.status));
        state.status_error = true;
    });
    rerender();
}

async fn handle_commit(response: &Response<'_>) {
    if response.status == ST_OK {
        let (name, target) = state::with_state(|state| {
            state
                .upload
                .as_ref()
                .map(|u| (u.name.clone(), u.target.clone()))
                .unwrap_or_default()
        });
        state::with_state(|state| {
            if let Some(u)=&state.upload{if let Some(clock)=u.clock{state.speed=speed_text(u.data.len() as u32,clock);}}
            state.upload = None;
            if state.preview.as_ref().is_some_and(|p| p.path == target) {
                state.preview = None;
            }
            state.status = format!("已上传 {}", name);
            state.status_error = false;
        });
    } else {
        state::with_state(|state| {
            state.upload = None;
            state.status = format!("上传失败：{}", status_text(response.status));
            state.status_error = true;
        });
    }
    rerender();
    load_dir(state::current_path()).await;
}

async fn handle_preview_open(pending: Pending, response: &Response<'_>) {
    let Pending::PreviewOpen { path, name } = pending else {
        return;
    };
    if state::with_state(|state| state.preview.is_none()) {
        return;
    }
    if response.status != ST_OK || response.payload.len() < 4 {
        state::with_state(|state| {
            if let Some(preview) = state.preview.as_mut() {
                preview.loading = false;
                preview.error = Some(format!("打开失败：{}", status_text(response.status)));
            }
        });
        rerender();
        return;
    }
    let size = get_u32(response.payload, 0).unwrap_or(0);
    if size > 256 * 1024 {
        state::with_state(|state| {
            if let Some(preview) = state.preview.as_mut() {
                preview.loading = false;
                preview.error = Some("文件超过 256 KiB，暂不支持预览。".to_string());
            }
        });
        rerender();
        return;
    }
    state::with_state(|state| {
        if let Some(preview) = state.preview.as_mut() {
            preview.path = path;
            preview.name = name;
            preview.size = size;
            preview.data.clear();
            preview.text.clear();
            preview.error = None;
            preview.loading = true;
        }
    });
    if size == 0 {
        finish_preview_read();
    } else {
        request_preview_read(0).await;
    }
}

async fn handle_preview_read(pending: Pending, response: &Response<'_>) {
    let Pending::PreviewRead { offset } = pending else {
        return;
    };
    if state::with_state(|state| state.preview.is_none()) {
        return;
    }
    if response.status != ST_OK {
        state::with_state(|state| {
            if let Some(preview) = state.preview.as_mut() {
                preview.loading = false;
                preview.error = Some(format!("读取失败：{}", status_text(response.status)));
            }
        });
        rerender();
        return;
    }
    let (next, size) = state::with_state(|state| {
        let Some(preview) = state.preview.as_mut() else {
            return (0, 0);
        };
        preview.data.extend_from_slice(response.payload);
        (offset + response.payload.len() as u32, preview.size)
    });
    if next >= size {
        finish_preview_read();
    } else {
        request_preview_read(next).await;
    }
}

async fn handle_download_open(pending: Pending, response: &Response<'_>) {
    let Pending::DownloadOpen { session, .. } = pending else {
        return;
    };
    if state::with_state(|state| state.download.is_none()) {
        return;
    }
    if response.status != ST_OK || response.payload.len() < 4 {
        let _ = dialog::save_file_abort(session).await;
        state::with_state(|state| state.download = None);
        state::set_status(format!("打开下载失败：{}", status_text(response.status)), true);
        rerender();
        return;
    }
    let size = get_u32(response.payload, 0).unwrap_or(0);
    state::with_state(|state| {
        if let Some(download) = state.download.as_mut() {
            download.size = size;
            download.next_issue = 0;
            download.next_write = 0;
            download.written = 0;
            download.clock=Instant::now();download.last_ack=Instant::now();
        }
        state.status = "下载中…".to_string();
        state.status_error = false;
    });
    if size == 0 {
        finish_download().await;
        return;
    }
    fill_read_window().await;
    rerender();
}

async fn connection_lost(reason:&str){
    *HEARTBEAT.lock().unwrap_or_else(|e|e.into_inner())=None;
    if state::with_state(|s|s.upload.is_some()){cancel_upload_with(reason.into()).await;}
    if state::with_state(|s|s.download.is_some()){cancel_download().await;}
    if state::with_state(|s|s.benchmark.is_some()){end_benchmark(Some(reason.into()));}
    state::clear_connection();
    state::set_status(reason,true);rerender();
}

async fn heartbeat(){
    if !state::with_state(|s|s.connected){return;}
    let in_flight=*HEARTBEAT.lock().unwrap_or_else(|e|e.into_inner());
    if let Some((_,sent))=in_flight{
        if sent.elapsed().as_secs()>=10{connection_lost("手环心跳超时，已断开；请重新连接ChaosConnect。").await;}
        return;
    }
    let interval=if state::with_state(|s|s.dev_authorization.is_some()){1}else{3};
    if LAST_HEARTBEAT.lock().unwrap_or_else(|e|e.into_inner()).is_some_and(|t|t.elapsed().as_secs()<interval){return;}
    let req=next_req();let now=Instant::now();
    *HEARTBEAT.lock().unwrap_or_else(|e|e.into_inner())=Some((req,now));
    *LAST_HEARTBEAT.lock().unwrap_or_else(|e|e.into_inner())=Some(now);
    if let Err(error)=send_req(OP_PING,req,&[]).await{
        connection_lost(&format!("心跳发送失败：{error}")).await;
    }
}

async fn apply_device_status(status:DeviceStatus){
    if !status.connected{connection_lost("手环已结束ChaosConnect会话，请重新连接。").await;return;}
    let mut changed=false;
    state::with_state(|s|{
        changed=s.dev_mode!=status.developer||s.dev_policy!=status.policy;
        let revoked=s.dev_mode&&!status.developer;
        s.dev_mode=status.developer;s.dev_policy=status.policy;
        if let Some((token,started))=s.dev_authorization{
            if token!=0&&status.token==token&&status.authorization==1{return;}
            if token==0&&started.elapsed().as_secs()<8{return;}
            if token==0&&status.authorization==1&&status.token!=0{
                s.dev_authorization=Some((status.token,Instant::now()));
                s.pending=None;s.pending_req=0;
                s.status="手环正在等待授权，请点亮屏幕并确认。".into();s.status_error=false;changed=true;return;
            }
            s.dev_authorization=None;s.busy=false;changed=true;
            let (message,error)=if token==0{
                ("操作应答超时，已同步手环实际状态。",true)
            }else if status.token!=token||status.authorization==0{
                ("手环已取消本次授权请求。",true)
            }else{match status.authorization{
                2 if status.developer=>("手环已允许，开发者模式已开启。",false),
                3=>("手环已拒绝，开发者模式未开启。",true),
                4=>("手环授权超时，开发者模式未开启。",true),
                _=>("手环未开启开发者模式，已同步实际状态。",true),
            }};
            s.status=message.into();s.status_error=error;
            if matches!(s.pending,Some(Pending::SetDev{..})){s.pending=None;s.pending_req=0;}
        }else if revoked{s.status="手环已撤销开发者模式。".into();s.status_error=false;}
    });
    if changed{rerender();}
}

async fn handle_set_dev(pending: Pending, response: &Response<'_>) {
    let Pending::SetDev { enabled } = pending else {
        return;
    };
    if enabled&&response.status==ST_PENDING{
        let Some(token)=get_u32(response.payload,0).filter(|t|*t!=0)else{
            state::with_state(|s|{s.busy=false;s.dev_authorization=None;});
            state::set_status("手环返回了无效授权编号，未开启开发者模式。",true);rerender();return;
        };
        state::with_state(|s|{s.dev_authorization=Some((token,Instant::now()));s.status="请点亮手环屏幕，确认是否允许启用开发者模式（60秒内有效）。".into();s.status_error=false;});
        rerender();return;
    }
    state::with_state(|s|{s.busy=false;s.dev_authorization=None;});
    if response.status != ST_OK {
        state::set_status(
            format!("切换开发者模式失败：{}", status_text(response.status)),
            true,
        );
        rerender();
        return;
    }
    state::with_state(|state| {
        state.dev_mode = enabled;
        state.status = if enabled {
            "开发者模式已开启：可写 /data 以外路径（受保护目录除外）".to_string()
        } else {
            "开发者模式已关闭：只能写 /data".to_string()
        };
        state.status_error = false;
    });
    rerender();
}

impl event::Guest for ChaosConnectPlugin {
    async fn on_event(event_type: EventType, event_payload: String) -> String {
        match event_type {
            EventType::DeviceAction => {
                // Start host device access only after the user first requests it.
                if DEVICES_REQUESTED.load(Ordering::SeqCst) {
                    refresh_devices().await;
                }
            }
            EventType::Timer => {
                let timer_name = timer_payload(&event_payload);
                if timer_name=="benchmark"{drive_benchmark().await;} else if timer_name == "upload" {
                    drive_upload().await;
                } else if timer_name=="transfer-watchdog"{
                    heartbeat().await;
                    let upload_expired=state::with_state(|s|s.upload.as_ref().is_some_and(|u|u.last_ack.elapsed().as_secs()>=15));
                    if upload_expired{cancel_upload_with("等待上传或提交应答超时，请刷新目录核对结果。".into()).await;}
                    let expired=state::with_state(|s|s.download.as_ref().is_some_and(|d|d.last_ack.elapsed().as_secs()>=15));
                    if expired{cancel_download().await;state::set_status("下载等待设备应答超时，已取消。",true);rerender();}
                } else if timer_name == "hello-timeout" {
                    let pending = state::with_state(|state| {
                        if matches!(state.pending, Some(Pending::Hello)) {
                            state.pending = None;
                            state.busy = false;
                            true
                        } else {
                            false
                        }
                    });
                    if pending {
                        state::set_status(
                            "设备没有响应。请确认Chaos文件桥已开启，且AstroBox版本支持通用设备传输。",
                            true,
                        );
                        rerender();
                    }
                }
            }
            EventType::TransportPacket => receive_packet(&event_payload).await,
            EventType::PluginMessage
            | EventType::InterconnectMessage
            | EventType::ProviderAction
            | EventType::DeeplinkAction => {}
        }
        String::new()
    }

    async fn on_ui_event(event_id: String, _ui_event: event::UiEvent, event_payload: String) -> String {
        handle_ui_event(&event_id, &event_payload).await;
        String::new()
    }

    async fn on_ui_render(element_id: String) {
        state::with_state(|state| state.root = Some(element_id));
        rerender();
    }

    async fn on_card_render(_card_id: String) {}
}

async fn receive_packet(encoded:&str){
    let Ok(packet)=base64::engine::general_purpose::STANDARD.decode(encoded)else{return;};
    {let mut q=RECEIVE_QUEUE.lock().unwrap_or_else(|e|e.into_inner());
     if q.len()>=WINDOW*4 {state::set_status("设备应答队列溢出，请重新连接。",true);return;}
     q.push_back(packet);if RECEIVE_BUSY.swap(true,Ordering::SeqCst){return;}}
    loop{
        let next={let mut q=RECEIVE_QUEUE.lock().unwrap_or_else(|e|e.into_inner());
            let next=q.pop_front();if next.is_none(){RECEIVE_BUSY.store(false,Ordering::SeqCst);}next};
        let Some(packet)=next else{break;};receive_frame(&packet).await;
    }
}
async fn receive_frame(packet:&[u8]){
    let Some((frame,id))=unwrap_packet(packet)else{return;};
    if id&0xffff0000!=PREFIX.load(Ordering::SeqCst){return;}
        if let Some((2,r))=parse_frame(frame){
            if r.op==EVENT_DEVICE_STATE&&r.req==0&&r.status==ST_OK{
                if let Some(status)=parse_device_status(r.payload){
                    if !status.connected{connection_lost("ChaosConnect已在手环停用，连接已断开。").await;}
                    else{apply_device_status(status).await;}
                }
                return;
            }
            if r.op==EVENT_WRITE_PROGRESS {if let Some(done)=get_u32(r.payload,0){state::with_state(|s|{if let Some(u)=s.upload.as_mut(){u.reported=u.reported.max(done);}});}}
            return;
        }
        let Some(response) = parse_response(frame) else {
            return;
        };
        if response.op==OP_PING{
            let expected={let mut h=HEARTBEAT.lock().unwrap_or_else(|e|e.into_inner());
                if h.is_some_and(|(req,_)|req==response.req){h.take();true}else{false}};
            if expected{
                if let Some(status)=parse_device_status(response.payload).filter(|_|response.status==ST_OK){apply_device_status(status).await;}
                else{connection_lost("设备状态应答无效，已断开，请同时更新Chaos和插件。").await;}
            }
            return;
        }
        if response.op==0x31{
            let chunk=state::with_state(|s|s.benchmark.as_mut().and_then(|b|b.inflight.remove(&response.req)));
            if let Some((offset,n))=chunk{
                if response.status!=ST_OK||response.payload.len()!=n||response.payload.iter().enumerate().any(|(i,v)|*v!=((offset as usize+i)*31)as u8){end_benchmark(Some("测速数据校验失败。".into()));return;}
                let done=state::with_state(|s|{let Some(b)=s.benchmark.as_mut()else{return false;};b.received+=n as u32;b.last_ack=Instant::now();b.received==BENCH_BYTES&&b.inflight.is_empty()});
                if done{end_benchmark(None);}else{drive_benchmark().await;}
            }return;
        }
        if response.op==OP_WRITE {
            let accepted=state::with_state(|s|{let Some(u)=s.upload.as_mut()else{return Ok(false);};
                if !u.inflight.contains_key(&response.req){return Ok(false);}
                if response.status!=ST_OK{return Err(());}
                u.acknowledge(response.req,get_u32(response.payload,0).ok_or(())?)
            });
            match accepted{
                Err(())=>{cancel_upload_with(format!("上传确认失败：{}",status_text(response.status))).await;return;}
                Ok(true)=>drive_upload().await,
                Ok(false)=>{}
            }return;
        }
        // Reads have their own in-flight map and no single pending slot.
        let read_offset = state::with_state(|state| {
            state
                .download
                .as_mut()
                .and_then(|download| download.inflight.remove(&response.req))
        });
        if let Some(offset) = read_offset {
            on_read_response(response.status, response.payload, offset).await;
            return;
        }
        let Some(pending) = take_pending(response.req) else {
            return;
        };
        match pending {
            Pending::Hello => handle_hello(&response).await,
            Pending::List { .. } => handle_list(pending, &response).await,
            Pending::Stat { .. } => handle_stat(pending, &response).await,
            Pending::Action { .. } => handle_action(pending, &response).await,
            Pending::UploadOpen => handle_upload_open(&response).await,
            Pending::Commit => handle_commit(&response).await,
            Pending::SetDev { .. } => handle_set_dev(pending, &response).await,
            Pending::DownloadOpen { .. } => handle_download_open(pending, &response).await,
            Pending::PreviewOpen { .. } => handle_preview_open(pending, &response).await,
            Pending::PreviewRead { .. } => handle_preview_read(pending, &response).await,
        }
}

impl lifecycle::Guest for ChaosConnectPlugin {
    async fn on_load() {
        // Report readiness without device/transport calls. Discovery and HELLO
        // start from the UI after the runtime is fully enabled.
    }
}

export!(ChaosConnectPlugin);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_finishes_without_calling_device_or_transport_imports() {
        use std::future::Future;
        use std::task::{Context, Poll, Waker};
        let mut startup = std::pin::pin!(<ChaosConnectPlugin as lifecycle::Guest>::on_load());
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(startup.as_mut().poll(&mut cx), Poll::Ready(())));
    }

    #[test]
    fn authorization_wait_allow_reject_revoke_and_cancel_follow_device_status() {
        fn apply(status:DeviceStatus){
            use std::future::Future;
            use std::task::{Context,Poll,Waker};
            let mut future=std::pin::pin!(apply_device_status(status));
            assert!(matches!(future.as_mut().poll(&mut Context::from_waker(Waker::noop())),Poll::Ready(())));
        }
        state::with_state(|s|{*s=state::State::default();s.connected=true;s.busy=true;s.dev_authorization=Some((19,Instant::now()));});
        let mut status=DeviceStatus{connected:true,developer:false,policy:2,authorization:1,token:19};
        apply(status);assert!(state::with_state(|s|s.busy&&!s.dev_mode&&s.dev_authorization.is_some()));
        status.authorization=2;status.developer=true;apply(status);
        assert!(state::with_state(|s|s.dev_mode&&!s.busy&&s.dev_authorization.is_none()));
        status.policy=1;status.developer=false;status.authorization=3;apply(status);
        assert!(!state::with_state(|s|s.dev_mode));
        for (token,auth) in [(19,3),(19,4),(20,2),(0,0)]{
            state::with_state(|s|{s.busy=true;s.dev_authorization=Some((19,Instant::now()));});
            status.token=token;status.authorization=auth;apply(status);
            assert!(state::with_state(|s|!s.dev_mode&&!s.busy&&s.dev_authorization.is_none()));
        }
        state::with_state(|s|{s.dev_mode=true;s.dev_authorization=Some((19,Instant::now()));s.busy=true;});
        state::clear_connection();
        assert!(state::with_state(|s|!s.connected&&!s.dev_mode&&!s.busy&&s.dev_authorization.is_none()));

        // Real receive path: only the matching session's valid stopped event
        // tears down the visible connection, without waiting for a heartbeat.
        fn receive(packet:&[u8]){
            use std::future::Future;
            use std::task::{Context,Poll,Waker};
            let mut future=std::pin::pin!(receive_frame(packet));
            assert!(matches!(future.as_mut().poll(&mut Context::from_waker(Waker::noop())),Poll::Ready(())));
        }
        PREFIX.store(0x11220000,Ordering::SeqCst);
        state::with_state(|s|{*s=state::State::default();s.connected=true;s.dev_mode=true;s.busy=true;s.dev_authorization=Some((99,Instant::now()));});
        let stopped=encode_frame(2,EVENT_DEVICE_STATE,ST_OK,0,&[0,0,2,0,0,0,0,0]);
        receive(&wrap_packet(&stopped,0x33440000));
        assert!(state::with_state(|s|s.connected));
        let mut corrupt=stopped.clone();*corrupt.last_mut().unwrap()^=1;
        receive(&wrap_packet(&corrupt,0x11220000));
        assert!(state::with_state(|s|s.connected));
        receive(&wrap_packet(&stopped,0x11220000));
        assert!(state::with_state(|s|!s.connected&&!s.dev_mode&&!s.busy&&s.dev_authorization.is_none()&&s.status.contains("停用")));
        PREFIX.store(0,Ordering::SeqCst);
    }

    #[test]
    fn list_payload_uses_one_byte_names() {
        let mut payload = Vec::new();
        put_u16(&mut payload, 2);
        put_u16(&mut payload, 0);
        put_u16(&mut payload, 3);
        for (name, is_dir, size, mtime) in [
            ("a.txt", 0u8, 5u32, 1u32),
            ("dir", 1u8, 0u32, 2u32),
            ("z", 0u8, 9u32, 3u32),
        ] {
            payload.push(name.len() as u8);
            payload.extend_from_slice(name.as_bytes());
            payload.push(is_dir);
            put_u32(&mut payload, size);
            put_u32(&mut payload, mtime);
        }
        let (total, offset, count, entries) = parse_list_entries(&payload).unwrap();
        assert_eq!((total, offset, count), (2, 0, 3));
        assert_eq!(entries[0].name, "a.txt");
        assert_eq!(entries[1].name, "dir");
        assert!(entries[1].is_dir);
        assert_eq!(entries[2].size, 9);
    }
}

#[cfg(test)]
mod timer_tests {
    use super::*;

    #[test]
    fn timer_payload_extracts_json_name() {
        assert_eq!(
            timer_payload(r#"{"kind":"interval","payload":"upload","timerId":3}"#),
            "upload"
        );
        assert_eq!(timer_payload("upload"), "upload");
        assert_eq!(
            timer_payload(r#"{"kind":"timeout","payload":"hello-timeout"}"#),
            "hello-timeout"
        );
    }
}
