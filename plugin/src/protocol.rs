//! ChaosConnect v1: private CRC-protected frames over the authenticated device transport.

// Unknown enum values survive prost but panic the host's pbjson trace logger.
// OTHER is defined in WearPacket.Type; ChaosConnect/CCN1 identifies our traffic.
pub const PB_TYPE: u32 = 0;
pub const HEADER: usize = 16;
pub const WINDOW: usize = 8;
pub const MAX_PAYLOAD: usize = CHUNK + 256;
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
pub const EVENT_DEVICE_STATE: u8 = 0xC2;

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
pub const ST_PENDING: u8 = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceStatus {
    pub connected: bool,
    pub developer: bool,
    pub policy: u8,
    pub authorization: u8,
    pub token: u32,
}
pub fn parse_device_status(data: &[u8]) -> Option<DeviceStatus> {
    if data.len()!=8||data[0]>1||data[1]>1||data[2]>2||data[3]>4{return None;}
    let token=get_u32(data,4)?;
    if (data[0]==0&&data[1]!=0)||(data[3]!=0&&token==0){return None;}
    Some(DeviceStatus{connected:data[0]!=0,developer:data[1]!=0,policy:data[2],authorization:data[3],token})
}

pub const CHUNK: usize = 2048;

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
    let len = u16::try_from(bytes.len()).unwrap_or(0);
    put_u16(out, len);
    out.extend_from_slice(bytes);
}

pub fn read_str(data: &[u8], off: usize) -> Option<(String, usize)> {
    let len = get_u16(data, off)? as usize;
    let start = off + 2;
    let bytes = data.get(start..start + len)?;
    Some((String::from_utf8_lossy(bytes).into_owned(), start + len))
}

pub fn encode_frame(flags:u8,op:u8,status:u8,req:u16,payload:&[u8])->Vec<u8>{
    assert!(payload.len()<=MAX_PAYLOAD);
    let mut out=Vec::with_capacity(HEADER+payload.len());
    out.extend_from_slice(b"CCN1");out.extend_from_slice(&[PROTO_VERSION,flags,op,status]);
    put_u16(&mut out,req);put_u16(&mut out,payload.len() as u16);put_u32(&mut out,0);out.extend_from_slice(payload);
    let crc=crc32(payload,crc32(&out[..12],0xffffffff))^0xffffffff;
    out[12..16].copy_from_slice(&crc.to_le_bytes());out
}
pub fn encode_request(op:u8,req:u16,payload:&[u8])->Vec<u8>{encode_frame(0,op,0,req,payload)}
pub struct Response<'a>{pub op:u8,pub req:u16,pub status:u8,pub payload:&'a [u8]}
pub fn parse_frame(frame:&[u8])->Option<(u8,Response<'_>)>{
    if frame.len()<HEADER||frame.get(..4)?!=b"CCN1"||frame[4]!=PROTO_VERSION||frame[5]>2||frame.len()!=HEADER+get_u16(frame,10)? as usize||frame.len()>HEADER+MAX_PAYLOAD{return None;}
    if get_u32(frame,12)?!=(crc32(&frame[HEADER..],crc32(&frame[..12],0xffffffff))^0xffffffff){return None;}
    Some((frame[5],Response{op:frame[6],req:get_u16(frame,8)?,status:frame[7],payload:&frame[HEADER..]}))
}
pub fn parse_response(frame:&[u8])->Option<Response<'_>>{let(flags,r)=parse_frame(frame)?;(flags==1).then_some(r)}
fn varint(out:&mut Vec<u8>,mut value:u32){while value>=128{out.push((value as u8&127)|128);value>>=7;}out.push(value as u8);}
fn bytes_field(out:&mut Vec<u8>,field:u32,value:&[u8]){varint(out,field<<3|2);varint(out,value.len() as u32);out.extend_from_slice(value);}
/// Known MessageContent bytes fields survive the host's prost decode/re-encode.
/// Use the defined OTHER enum; privacy is in the package name and CCN1 frame.
pub fn wrap_packet(frame:&[u8],id:u32)->Vec<u8>{
    let mut basic=Vec::new();bytes_field(&mut basic,1,b"ChaosConnect");bytes_field(&mut basic,2,&[]);
    let mut message=Vec::new();bytes_field(&mut message,1,&basic);bytes_field(&mut message,2,frame);
    let mut app=Vec::new();bytes_field(&mut app,9,&message);
    let mut out=Vec::new();varint(&mut out,8);varint(&mut out,PB_TYPE);varint(&mut out,16);varint(&mut out,id);bytes_field(&mut out,22,&app);out
}
fn read_var(p:&[u8],at:&mut usize)->Option<u32>{let mut v=0;for shift in (0..35).step_by(7){let b=*p.get(*at)?;*at+=1;if shift==28&&b&0xf0!=0{return None;}v|=((b&127) as u32)<<shift;if b&128==0{return Some(v);}}None}
fn field(p:&[u8],wanted:u32,wire:u32)->Option<(&[u8],u32)>{
    let mut at=0;let mut found=None;
    while at<p.len(){let tag=read_var(p,&mut at)?;if tag<8{return None;}
        let (data,value)=match tag&7{0=>( &p[0..0],read_var(p,&mut at)?),2=>{let n=read_var(p,&mut at)? as usize;let end=at.checked_add(n)?;let data=p.get(at..end)?;at=end;(data,n as u32)},1|5=>{at=at.checked_add(if tag&7==1{8}else{4})?;p.get(..at)?;(&p[0..0],0)},_=>return None};
        if tag>>3==wanted{if found.is_some()||tag&7!=wire{return None;}found=Some((data,value));}
    }found
}
pub fn unwrap_packet(p:&[u8])->Option<(&[u8],u32)>{
    if field(p,1,0)?.1!=PB_TYPE{return None;}let id=field(p,2,0)?.1;
    let app=field(p,22,2)?.0;let message=field(app,9,2)?.0;let basic=field(message,1,2)?.0;
    if field(basic,1,2)?.0!=b"ChaosConnect"{return None;}let frame=field(message,2,2)?.0;let(_,r)=parse_frame(frame)?;if r.req as u32!=id&0xffff{return None;}Some((frame,id))
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
        ST_UNSUPPORTED => "当前文件桥/固件不支持该操作",
        ST_TOO_LARGE => "文件大小超出限制",
        ST_PENDING => "等待手环授权",
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
        assert_eq!(&frame[..4], b"CCN1");
        assert_eq!(&frame[HEADER..], &[1,2,3]);
        let response = encode_frame(1, OP_LIST, ST_OK, 0x1234, &[9,8]);
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
    fn device_status_validates_authorization_tokens_and_closed_state() {
        for auth in 0..=4 {
            let token=if auth==0{0u32}else{19};
            let mut data=vec![1,if auth==2{1}else{0},2,auth];put_u32(&mut data,token);
            let status=parse_device_status(&data).unwrap();
            assert_eq!((status.authorization,status.token),(auth,token));
        }
        for bad in [vec![],vec![1],vec![0,1,2,0,0,0,0,0],vec![1,0,3,0,0,0,0,0],
                    vec![1,0,2,1,0,0,0,0],vec![1,0,2,5,1,0,0,0]] {
            assert!(parse_device_status(&bad).is_none());
        }
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

#[cfg(test)] mod connect_tests{
 use super::*;
 #[test] fn rejects_corruption_duplicates_and_truncation(){
  let f=encode_frame(1,OP_READ,0,7,&vec![19;CHUNK]);let p=wrap_packet(&f,0x56780007);
  assert_eq!(unwrap_packet(&p).unwrap(),(f.as_slice(),0x56780007));
  for n in 0..f.len(){assert!(parse_frame(&f[..n]).is_none());}
  for n in [0,4,5,7,8,10,12,16,f.len()-1]{let mut damaged=f.clone();damaged[n]^=1;assert!(parse_frame(&damaged).is_none());}
  let mut duplicate=p.clone();duplicate.extend_from_slice(&[8,0xc3,0x86,1]);assert!(unwrap_packet(&duplicate).is_none());
 }
}

#[cfg(test)] mod host_roundtrip_tests {
    use super::*;
    use prost::Message;
    use chaos_connect_host_pb_fixture::protocol::WearPacket;
    #[test] fn host_prost_reencode_keeps_known_type_and_binary_payload(){
        for n in [0,1,127,128,CHUNK,MAX_PAYLOAD]{
            let data=(0..n).map(|i|i as u8).collect::<Vec<_>>();
            let frame=encode_request(OP_WRITE,0x1234,&data);let packet=wrap_packet(&frame,0xabcd1234);
            let decoded=WearPacket::decode(packet.as_slice()).unwrap();
            assert_eq!(decoded.r#type,PB_TYPE as i32);assert_eq!(decoded.id,0xabcd1234);
            let encoded=decoded.encode_to_vec();assert_eq!(encoded,packet);
            assert_eq!(unwrap_packet(&encoded).unwrap(),(frame.as_slice(),0xabcd1234));
        }
    }
}

#[cfg(test)] mod wire_vectors {
    use super::*;
    #[test] fn shared_c_rust_wire_vector(){
        let values=include_str!("../tests/CHAOSCONNECT-V1.hex").lines().map(|line|{
            (0..line.len()).step_by(2).map(|i|u8::from_str_radix(&line[i..i+2],16).unwrap()).collect::<Vec<_>>()
        }).collect::<Vec<_>>();
        assert_eq!(encode_request(OP_HELLO,0x1234,&[]),values[0]);
        assert_eq!(wrap_packet(&values[0],0x12341234),values[1]);
    }
}

#[cfg(test)]
mod host_log_tests {
    use super::*;
    use chaos_connect_host_pb_fixture::protocol::{WearPacket, wear_packet};
    use prost::Message;

    #[test]
    fn old_private_enum_reproduces_host_json_log_failure() {
        let wire = wrap_packet(&encode_request(OP_HELLO, 1, &[]), 0x12340001);
        let mut packet = WearPacket::decode(wire.as_slice()).unwrap();
        packet.r#type = 0x4343;
        // Prost alone accepts it; pbjson (used by the host trace logger) does not.
        let packet = WearPacket::decode(packet.encode_to_vec().as_slice()).unwrap();
        let error = serde_json::to_string(&packet).unwrap_err().to_string();
        assert!(error.contains("Invalid variant 17219"), "{error}");
    }

    #[test]
    fn hello_and_data_survive_host_binary_and_json_logging() {
        assert_eq!(PB_TYPE, wear_packet::Type::Other as u32);
        for (flags, op, n) in [(0, OP_HELLO, 0), (1, OP_HELLO, 11),
                              (0, OP_WRITE, CHUNK + 4), (1, OP_READ, CHUNK),
                              (2, EVENT_WRITE_PROGRESS, 8), (0, OP_WRITE, MAX_PAYLOAD)] {
            let data = (0..n).map(|i| i as u8).collect::<Vec<_>>();
            let frame = encode_frame(flags, op, ST_OK, 0x1234, &data);
            let wire = wrap_packet(&frame, 0xabcd1234);
            let decoded = WearPacket::decode(wire.as_slice()).unwrap();
            let log_json = serde_json::to_string(&decoded).unwrap();
            let restored: WearPacket = serde_json::from_str(&log_json).unwrap();
            for packet in [decoded, restored] {
                let encoded = packet.encode_to_vec();
                assert_eq!(encoded, wire);
                assert_eq!(unwrap_packet(&encoded).unwrap(), (frame.as_slice(), 0xabcd1234));
            }
        }
    }

    #[test]
    fn other_type_still_requires_our_namespace_frame_and_request_id() {
        let frame = encode_request(OP_HELLO, 7, &[]);
        let wire = wrap_packet(&frame, 0x12340007);
        let mut other_app = wire.clone();
        let name_at = other_app.windows(12).position(|v| v == b"ChaosConnect").unwrap();
        other_app[name_at] = b'X';
        assert!(unwrap_packet(&other_app).is_none());
        assert!(unwrap_packet(&wrap_packet(&frame, 0x12340008)).is_none());
        assert!(unwrap_packet(&[8, 0, 16, 0]).is_none());
        let mut corrupted = wire;
        *corrupted.last_mut().unwrap() ^= 1;
        assert!(unwrap_packet(&corrupted).is_none());
    }
}
