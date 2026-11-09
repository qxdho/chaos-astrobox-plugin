use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

use crate::protocol;

pub const PAGE_SIZE: usize = 80;
pub const MAX_ENTRIES: usize = 800;
pub const MAX_UPLOAD: usize = 32 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
    Upload,
    Download,
}

#[derive(Clone)]
pub struct TransferInfo {
    pub kind: TransferKind,
    pub name: String,
    pub target: String,
    pub done: u32,
    pub total: u32,
}

#[derive(Clone, Default)]
pub struct DeviceInfo {
    pub addr: String,
    pub name: String,
}

#[derive(Clone, Default)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub size: u32,
    pub mtime: u32,
}

#[derive(Clone, Default)]
pub struct Preview {
    pub path: String,
    pub name: String,
    pub size: u32,
    pub data: Vec<u8>,
    pub text: String,
    pub loading: bool,
    pub editing: bool,
    pub error: Option<String>,
}

#[derive(Clone, Default)]
pub struct Detail {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u32,
    pub mtime: u32,
}

pub struct Upload {
    pub target: String,
    pub name: String,
    pub data: Vec<u8>,
    pub offset: usize,
    pub crc: u32,
    pub overwrite: bool,
    pub started: bool,
    pub timer: Option<u64>,
    pub reported: u32,
}

pub struct Download {
    pub session: u64,
    pub path: String,
    pub name: String,
    pub size: u32,
    pub next_issue: u32,
    pub next_write: u32,
    pub received: BTreeMap<u32, Vec<u8>>,
    pub inflight: HashMap<u16, u32>,
    pub written: u32,
    pub closing: bool,
}

pub enum Pending {
    Hello,
    List {
        path: String,
        offset: u32,
        acc: Vec<Entry>,
        total: u32,
    },
    Stat {
        path: String,
    },
    Action {
        refresh: bool,
    },
    UploadOpen,
    Commit,
    SetDev {
        enabled: bool,
    },
    DownloadOpen {
        session: u64,
        path: String,
        name: String,
    },
    PreviewOpen {
        path: String,
        name: String,
    },
    PreviewRead {
        offset: u32,
    },
}

pub struct State {
    pub root: Option<String>,
    pub devices: Vec<DeviceInfo>,
    pub addr: String,
    pub connected: bool,
    pub dev_mode: bool,
    pub path: String,
    pub entries: Vec<Entry>,
    pub total: u32,
    pub page: usize,
    pub loading: bool,
    pub status: String,
    pub status_error: bool,
    pub detail: Option<Detail>,
    pub pending: Option<Pending>,
    pub pending_req: u16,
    pub upload: Option<Upload>,
    pub download: Option<Download>,
    pub preview: Option<Preview>,
    pub busy: bool,
    pub transfer: Option<TransferInfo>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            root: None,
            devices: Vec::new(),
            addr: String::new(),
            connected: false,
            dev_mode: false,
            path: "/data".to_string(),
            entries: Vec::new(),
            total: 0,
            page: 0,
            loading: false,
            status: "请选择已连接的手环，并确认 Canopus 模块 band_files 已启用。".to_string(),
            status_error: false,
            detail: None,
            pending: None,
            pending_req: 0,
            upload: None,
            download: None,
            preview: None,
            busy: false,
            transfer: None,
        }
    }
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

pub fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get_or_insert_with(State::default);
    f(state)
}

pub fn snapshot() -> State {
    with_state(|state| State {
        root: state.root.clone(),
        devices: state.devices.clone(),
        addr: state.addr.clone(),
        connected: state.connected,
        dev_mode: state.dev_mode,
        path: state.path.clone(),
        entries: state.entries.clone(),
        total: state.total,
        page: state.page,
        loading: state.loading,
        status: state.status.clone(),
        status_error: state.status_error,
        detail: state.detail.clone(),
        pending: None,
        pending_req: state.pending_req,
        upload: None,
        download: None,
        preview: state.preview.as_ref().map(|preview| Preview {
            path: preview.path.clone(),
            name: preview.name.clone(),
            size: preview.size,
            data: preview.data.clone(),
            text: preview.text.clone(),
            loading: preview.loading,
            editing: preview.editing,
            error: preview.error.clone(),
        }),
        busy: state.busy,
        transfer: state.upload.as_ref().map(|upload| TransferInfo {
            kind: TransferKind::Upload,
            name: upload.name.clone(),
            target: upload.target.clone(),
            done: upload.offset as u32,
            total: upload.data.len() as u32,
        }).or_else(|| state.download.as_ref().map(|download| TransferInfo {
            kind: TransferKind::Download,
            name: download.name.clone(),
            target: "本地".to_string(),
            done: download.written,
            total: download.size,
        })),
    })
}

pub fn set_status(text: impl Into<String>, error: bool) {
    with_state(|state| {
        state.status = text.into();
        state.status_error = error;
    });
}

pub fn selected_addr() -> String {
    with_state(|state| state.addr.clone())
}

pub fn is_connected() -> bool {
    with_state(|state| state.connected)
}

pub fn reset_path(path: &str) {
    with_state(|state| {
        state.path = path.to_string();
        state.entries.clear();
        state.total = 0;
        state.page = 0;
        state.detail = None;
    });
}

pub fn clear_connection() {
    with_state(|state| {
        state.connected = false;
        state.entries.clear();
        state.total = 0;
        state.detail = None;
        state.upload = None;
        state.download = None;
        state.pending = None;
        state.pending_req = 0;
    });
}

pub fn current_path() -> String {
    with_state(|state| state.path.clone())
}

pub fn progress_text(done: u32, total: u32) -> String {
    if total == 0 {
        return "0 B".to_string();
    }
    format!(
        "{} / {}  ({}%)",
        protocol::format_size(done),
        protocol::format_size(total),
        (done as u64 * 100 / total as u64).min(100)
    )
}
