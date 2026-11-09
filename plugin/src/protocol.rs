//! Binary protocol shared with the Canopus `band_files` module.

pub const MODULE_ID: &str = "band_files";
pub const PROTO_VERSION: u8 = 1;

pub const OP_HELLO: u8 = 0x01;
pub const OP_LIST: u8 = 0x02;
pub const OP_STAT: u8 = 0x03;
pub const OP_MKDIR: u8 = 0x04;
pub const OP_REMOVE: u8 = 0x05;
pub const OP_RMDIR: u8 = 0x06;
pub const OP_RENAME: u8 = 0x07;
pub const OP_COPY: u8 = 0x08;
pub const OP_OPEN_READ: u8 = 0x10;
pub const OP_READ: u8 = 0x11;
pub const OP_CLOSE_READ: u8 = 0x12;
pub const OP_OPEN_WRITE: u8 = 0x20;
pub const OP_WRITE: u8 = 0x21;
pub const OP_COMMIT: u8 = 0x22;
pub const OP_ABORT: u8 = 0x23;
pub const OP_PING: u8 = 0x30;
pub const OP_SET_DEV: u8 = 0x40;
pub const OP_GET_DEV: u8 = 0x41;

pub const EVENT_WRITE_PROGRESS: u8 = 0xC1;

pub const ST_OK: u8 = 0;
pub const ST_BAD: u8 = 1;
pub const ST_NOTFOUND: u8 = 2;
pub const ST_ACCESS: u8 = 3;
pub const ST_EXIST: u8 = 4;
pub const ST_NOTDIR: u8 = 5;
pub const ST_ISDIR: u8 = 6;
pub const ST_NOSPACE: u8 = 7;
pub const ST_BUSY: u8 = 8;
pub const ST_IO: u8 = 9;
pub const ST_CRC: u8 = 10;
pub const ST_SEQUENCE: u8 = 11;
pub const ST_UNSUPPORTED: u8 = 12;
pub const ST_TOO_LARGE: u8 = 13;

pub const CHUNK: usize = 448;

pub fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub fn get_u16(data: &[u8], off: usize) -> Option<u16> {
    let bytes = data.get(off..off + 2)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

pub fn get_u32(data: &[u8], off: usize) -> Option<u32> {
    let bytes = data.get(off..off + 4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

pub fn push_str(out: &mut Vec<u8>, value: &str) {
    let bytes = value.as_bytes();
    let len = bytes.len().min(255) as u16;
    put_u16(out, len);
    out.extend_from_slice(&bytes[..len as usize]);
}

pub fn read_str(data: &[u8], off: usize) -> Option<(String, usize)> {
    let len = get_u16(data, off)? as usize;
    let start = off + 2;
    let bytes = data.get(start..start + len)?;
    Some((String::from_utf8_lossy(bytes).into_owned(), start + len))
}

pub fn encode_request(op: u8, req: u16, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(3 + payload.len());
    out.push(op);
    put_u16(&mut out, req);
    out.extend_from_slice(payload);
    out
}

pub struct Response<'a> {
    pub op: u8,
    pub req: u16,
    pub status: u8,
    pub payload: &'a [u8],
}

pub fn parse_response(frame: &[u8]) -> Option<Response<'_>> {
    if frame.len() < 4 {
        return None;
    }
    let op = frame[0];
    if op & 0x80 == 0 {
        return None;
    }
    Some(Response {
        op: op & 0x7F,
        req: u16::from_le_bytes([frame[1], frame[2]]),
        status: frame[3],
        payload: &frame[4..],
    })
}

pub fn status_text(status: u8) -> &'static str {
    match status {
        ST_OK => "成功",
        ST_BAD => "请求参数错误",
        ST_NOTFOUND => "文件或目录不存在",
        ST_ACCESS => "没有权限（或路径受保护）",
        ST_EXIST => "目标已存在",
        ST_NOTDIR => "不是目录",
        ST_ISDIR => "目标是目录",
        ST_NOSPACE => "空间不足",
        ST_BUSY => "目录非空或设备忙",
        ST_IO => "文件系统读写失败",
        ST_CRC => "校验失败，文件未写入",
        ST_SEQUENCE => "传输顺序错误",
        ST_UNSUPPORTED => "当前模块/固件不支持该操作",
        ST_TOO_LARGE => "文件大小超出限制",
        _ => "未知错误",
    }
}

pub fn crc32(data: &[u8], mut crc: u32) -> u32 {
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

pub fn parent(path: &str) -> String {
    if path == "/" {
        return "/".to_string();
    }
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/".to_string(),
        Some(pos) => trimmed[..pos].to_string(),
        None => "/".to_string(),
    }
}

pub fn join(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else {
        format!("{dir}/{name}")
    }
}

pub fn file_name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

pub fn format_size(size: u32) -> String {
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{:.2} MB", size as f64 / (1024.0 * 1024.0))
    }
}

pub fn format_time(seconds: u32) -> String {
    if seconds == 0 {
        return "-".to_string();
    }
    let days = (seconds / 86_400) as i64;
    let rem = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// Howard Hinnant's civil-from-days, for a UTC-ish timestamp.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let frame = encode_request(OP_LIST, 0x1234, &[1, 2, 3]);
        assert_eq!(frame, vec![OP_LIST, 0x34, 0x12, 1, 2, 3]);
        let response = [OP_LIST | 0x80, 0x34, 0x12, ST_OK, 9, 8];
        let parsed = parse_response(&response).unwrap();
        assert_eq!(parsed.op, OP_LIST);
        assert_eq!(parsed.req, 0x1234);
        assert_eq!(parsed.status, ST_OK);
        assert_eq!(parsed.payload, &[9, 8]);
    }

    #[test]
    fn crc_matches_standard_vector() {
        assert_eq!(crc32(b"123456789", 0xFFFF_FFFF) ^ 0xFFFF_FFFF, 0xCBF4_3926);
    }

    #[test]
    fn paths() {
        assert_eq!(parent("/data/foo"), "/data");
        assert_eq!(parent("/data"), "/");
        assert_eq!(parent("/"), "/");
        assert_eq!(join("/", "a"), "/a");
        assert_eq!(join("/data", "a"), "/data/a");
        assert_eq!(file_name("/data/a.bin"), "a.bin");
    }

    #[test]
    fn time_formatting() {
        assert_eq!(format_time(0), "-");
        assert_eq!(format_time(1_700_000_000), "2023-11-14 22:13");
    }
}
