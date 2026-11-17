//! Bounded text console for the module's independent CCN1 PTY lease.
use std::time::Instant;
use crate::protocol::{get_u32,Response};
#[derive(Clone)]
pub struct Terminal {
 pub visible:bool,pub supported:bool,pub lease:u32,pub state:u8,pub output:String,pub input:String,pub message:String,
 pub pending:Option<(u16,u8,Instant)>,pub closing:bool,pub last_poll:Option<Instant>,
 tail:Vec<u8>,escape:u8,
}
impl Default for Terminal {fn default()->Self{Self{visible:false,supported:false,lease:0,state:0,output:String::new(),input:String::new(),message:"需要手环允许插件 NSH终端权限".into(),pending:None,closing:false,last_poll:None,tail:Vec::new(),escape:0}}}
impl Terminal {
 pub fn busy(&self)->bool{self.pending.is_some()}
 pub fn active(&self)->bool{self.lease!=0}
 pub fn apply(&mut self,r:&Response<'_>)->Result<(),String>{
  let Some((id,op,_))=self.pending else{return Ok(());};if id!=r.req||op!=r.op{return Ok(());}self.pending=None;
  if r.status!=0{self.lease=0;self.closing=false;self.message=format!("NSH：{}",crate::protocol::status_text(r.status));return Err(self.message.clone());}
  if op==0x51||op==0x53 {if !r.payload.is_empty(){return Err("NSH操作应答无效".into());}if op==0x53{self.lease=0;self.state=0;self.closing=false;self.message="终端已停止".into();}return Ok(());}
  if r.payload.len()<9{return Err("NSH状态应答截断".into());}
  let lease=get_u32(r.payload,0).ok_or("NSH缺少会话编号")?;
  let state=r.payload[4];if lease==0||state>5||(op!=0x50&&lease!=self.lease){return Err("NSH会话已失效".into());}
  self.lease=lease;self.state=state;
  let error=get_u32(r.payload,5).unwrap_or(0)as i32;
  self.message=format!("{}{}",["未启动","启动中","运行中","已退出","启动失败","停止中"][state as usize],if error==0{String::new()}else{format!(" · {error}")});
  self.append(&r.payload[9..]);if state==3||state==4{if r.payload.len()==9{self.closing=true;}}Ok(())
 }
 pub fn append(&mut self,bytes:&[u8]){
  self.tail.extend_from_slice(bytes);
  let mut text=String::new();loop {match std::str::from_utf8(&self.tail){Ok(s)=>{text.push_str(s);self.tail.clear();break;}Err(e)=>{let valid=e.valid_up_to();text.push_str(std::str::from_utf8(&self.tail[..valid]).unwrap_or(""));if let Some(n)=e.error_len(){text.push('\u{fffd}');self.tail.drain(..valid+n);}else{self.tail.drain(..valid);break;}}}}
  for ch in text.chars(){match self.escape {
   1=>self.escape=match ch{'['=>2,']'=>3,_=>0},
   2=>if ('@'..='~').contains(&ch){self.escape=0},
   3=>if ch=='\u{7}'{self.escape=0}else if ch=='\u{1b}'{self.escape=4},
   4=>self.escape=if ch=='\\'{0}else{3},
   _=>match ch{'\u{1b}'=>self.escape=1,'\u{8}'|'\u{7f}'=>{self.output.pop();},'\r'=>{},'\n'|'\t'=>self.output.push(ch),c if !c.is_control()=>self.output.push(c),_=>{}}
  }}
  if self.output.len()>32768{let mut at=self.output.len()-32768;while !self.output.is_char_boundary(at){at+=1;}self.output.drain(..at);}
 }
}
#[cfg(test)]mod tests{use super::*;
 #[test]fn split_utf8_ansi_and_bounded_history(){let mut t=Terminal::default();t.append(&[0xe4,0xbd]);t.append(&[0xa0]);t.append(b"\x1b[31mhi\x1b[0m\x1b]title\x07\n");assert_eq!(t.output,"你hi\n");t.append("字".repeat(20000).as_bytes());assert!(t.output.len()<=32768);}
 #[test]fn ignores_old_request_and_lease(){let mut t=Terminal::default();t.pending=Some((5,0x50,Instant::now()));let p=[3,0,0,0,2,0,0,0,0];t.apply(&Response{op:0x50,req:4,status:0,payload:&p}).unwrap();assert_eq!(t.lease,0);t.apply(&Response{op:0x50,req:5,status:0,payload:&p}).unwrap();assert_eq!(t.lease,3);t.pending=Some((6,0x52,Instant::now()));let p=[4,0,0,0,2,0,0,0,0];assert!(t.apply(&Response{op:0x52,req:6,status:0,payload:&p}).is_err());}
 #[test]fn revoked_permission_stops_console(){let mut t=Terminal::default();t.lease=8;t.pending=Some((3,0x52,Instant::now()));assert!(t.apply(&Response{op:0x52,req:3,status:3,payload:&[]}).is_err());assert_eq!(t.lease,0);}
}
