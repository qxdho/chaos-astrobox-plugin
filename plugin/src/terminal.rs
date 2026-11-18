//! Bounded NSH console. Read polling never owns the command editor or drops input.
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use crate::protocol::{get_u32, Response};
const OPEN_WAIT: Duration = Duration::from_secs(15);
const RETRY_DELAY: Duration = Duration::from_millis(500);
const WRITE_WAIT: Duration = Duration::from_secs(5);
const MAX_COMMANDS: usize = 4;

#[derive(Clone)]
pub struct Terminal {
    pub visible: bool, pub supported: bool, pub lease: u32, pub state: u8,
    pub output: String, pub input: String, pub message: String,
    pub pending: Option<(u16,u8,Instant)>, pub closing: bool,
    pub last_poll: Option<Instant>, pub render_dirty: bool,
    opening: Option<Instant>, open_retry: Option<Instant>,
    commands: VecDeque<Vec<u8>>, write_retry: Option<Instant>, write_busy_since: Option<Instant>,
    pub editing: bool,
    tail: Vec<u8>, escape: u8,
}
impl Default for Terminal {
    fn default() -> Self { Self {
        visible:false, supported:false, lease:0, state:0, output:String::new(), input:String::new(),
        message:"连接ChaosConnect后即可启动NSH终端".into(), pending:None, closing:false, last_poll:None,
        opening:None, open_retry:None, commands:VecDeque::new(), write_retry:None, write_busy_since:None,
        editing:false, render_dirty:false, tail:Vec::new(), escape:0,
    }}
}
impl Terminal {
    pub fn busy(&self)->bool { self.pending.is_some() }
    pub fn active(&self)->bool { self.lease!=0 }
    pub fn starting(&self)->bool { self.opening.is_some() }
    pub fn needs_timer(&self)->bool { self.active()||self.busy()||self.starting()||self.render_dirty }
    pub fn can_send(&self)->bool { self.active() && matches!(self.state,1|2) && !self.closing && self.commands.len()<MAX_COMMANDS }
    pub fn queued(&self)->usize { self.commands.len() }
    pub fn edit(&mut self,value:String,_now:Instant) { self.input=value.chars().take(2048).collect(); }
    pub fn finish_edit(&mut self,value:Option<String>,now:Instant) {self.editing=false;if let Some(value)=value{self.edit(value,now);}}
    pub fn defer_render(&self,_now:Instant)->bool { self.visible && self.editing }
    pub fn begin_open(&mut self,now:Instant) {
        if self.active()||self.busy()||self.starting(){return;}
        self.opening=Some(now);self.open_retry=Some(now);self.closing=false;
        self.message="正在启动NSH终端…".into();
    }
    pub fn stop(&mut self) {
        self.closing=true;self.opening=None;self.open_retry=None;self.commands.clear();self.write_retry=None;self.write_busy_since=None;
    }
    pub fn enqueue(&mut self,interrupt:bool,chunk:usize)->Result<(),String> {
        if !self.can_send(){return Err("NSH尚未运行、正在停止或待发送队列已满".into());}
        let bytes=if interrupt{vec![3]}else{
            if self.input.trim().is_empty(){return Err("请输入NSH命令".into());}
            let mut bytes=self.input.as_bytes().to_vec();bytes.push(b'\n');bytes
        };
        if bytes.len()>chunk||bytes.iter().any(|b|*b==0||*b==4){return Err("输入超过协商长度或包含不支持的字符".into());}
        if interrupt {
            // Never move a command whose WRITE acknowledgement is still pending.
            let index=if self.pending.is_some_and(|(_,op,_)|op==0x51){1.min(self.commands.len())}else{0};
            self.commands.insert(index,bytes);
        }else{self.commands.push_back(bytes);self.input.clear();}
        Ok(())
    }
    pub fn next_action(&mut self,now:Instant,chunk:usize)->Option<(u8,Vec<u8>)> {
        if self.busy(){return None;}
        if let Some(start)=self.opening {
            if now.duration_since(start)>=OPEN_WAIT {
                self.opening=None;self.open_retry=None;
                self.message="上次NSH终端尚未回收完成，请稍后再试".into();return None;
            }
            if self.open_retry.is_none_or(|at|now>=at){self.open_retry=None;return Some((0x50,Vec::new()));}
            return None;
        }
        if !self.active(){return None;}
        let mut p=self.lease.to_le_bytes().to_vec();
        if self.closing{return Some((0x53,p));}
        if let Some(start)=self.write_busy_since {
            if now.duration_since(start)>=WRITE_WAIT {
                self.commands.clear();self.write_busy_since=None;self.write_retry=None;
                self.message="NSH输入队列持续繁忙，命令未被接收，请重新输入".into();
            }
        }
        if self.write_retry.is_none_or(|at|now>=at) {
            if let Some(bytes)=self.commands.front(){p.extend_from_slice(bytes);return Some((0x51,p));}
        }
        let interval=Duration::from_millis(if self.visible{250}else{1000});
        if self.last_poll.is_some_and(|at|now.duration_since(at)<interval){return None;}
        self.last_poll=Some(now);p.extend_from_slice(&(chunk as u16).to_le_bytes());Some((0x52,p))
    }
    pub fn send_failed(&mut self,error:&str) {
        self.pending=None;self.lease=0;self.stop();self.editing=false;
        self.message=format!("NSH发送失败，命令不会自动重发：{error}");
    }
    pub fn apply(&mut self,r:&Response<'_>)->Result<(),String>{self.apply_at(r,Instant::now())}
    pub fn apply_at(&mut self,r:&Response<'_>,now:Instant)->Result<(),String> {
        let Some((id,op,_))=self.pending else{return Ok(());};
        if id!=r.req||op!=r.op{return Ok(());}self.pending=None;
        if r.status==8 && op==0x50 {
            if self.closing {self.opening=None;self.open_retry=None;self.message="终端启动已取消".into();return Ok(());}
            self.opening.get_or_insert(now);self.open_retry=Some(now+RETRY_DELAY);
            self.message="上次NSH终端正在回收，等待释放…".into();return Ok(());
        }
        if r.status==8 && op==0x51 {
            // BUSY is an explicit rejection: the module's TX.push_all accepted no bytes.
            // Only that rejection may be retried; transport loss never replays a command.
            if !self.closing {self.write_busy_since.get_or_insert(now);self.write_retry=Some(now+RETRY_DELAY);}
            self.message="NSH输入队列暂忙，等待接收…".into();return Ok(());
        }
        if r.status!=0 {
            self.lease=0;self.stop();self.editing=false;
            self.message=format!("NSH：{}",crate::protocol::status_text(r.status));return Err(self.message.clone());
        }
        if op==0x51||op==0x53 {
            if !r.payload.is_empty(){return Err("NSH操作应答无效".into());}
            if op==0x51{self.commands.pop_front();self.write_retry=None;self.write_busy_since=None;}
            else{self.lease=0;self.state=0;self.stop();self.closing=false;self.message="终端已停止".into();}
            return Ok(());
        }
        if r.payload.len()<9{return Err("NSH状态应答截断".into());}
        let lease=get_u32(r.payload,0).ok_or("NSH缺少会话编号")?;
        let state=r.payload[4];if lease==0||state>5||(op!=0x50&&lease!=self.lease){return Err("NSH会话已失效".into());}
        self.lease=lease;self.state=state;self.opening=None;self.open_retry=None;
        let error=get_u32(r.payload,5).unwrap_or(0)as i32;
        self.message=format!("{}{}",["未启动","启动中","运行中","已退出","启动失败","停止中"][state as usize],if error==0{String::new()}else{format!(" · {error}")});
        self.append(&r.payload[9..]);
        if (state==3||state==4)&&r.payload.len()==9{self.stop();}
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    fn running()->Terminal {let mut t=Terminal::default();t.visible=true;t.lease=7;t.state=2;t}
    fn reply(t:&mut Terminal,op:u8,status:u8,payload:&[u8],now:Instant){
        t.pending=Some((5,op,now));t.apply_at(&Response{op,req:5,status,payload},now).unwrap();
    }
    #[test] fn read_in_flight_does_not_disable_send_or_drop_a_command(){
        let now=Instant::now();let mut t=running();t.pending=Some((5,0x52,now));
        t.edit("ls /data".into(),now);assert!(t.can_send());t.enqueue(false,2048).unwrap();
        assert_eq!(t.queued(),1);assert!(t.input.is_empty());assert!(t.next_action(now,2048).is_none());
        t.apply_at(&Response{op:0x52,req:5,status:0,payload:&[7,0,0,0,2,0,0,0,0]},now).unwrap();
        let (op,p)=t.next_action(now,2048).unwrap();assert_eq!(op,0x51);assert_eq!(&p[4..],b"ls /data\n");
    }
    #[test] fn acknowledged_write_is_removed_once_and_never_replayed(){
        let now=Instant::now();let mut t=running();t.input="echo ok".into();t.enqueue(false,2048).unwrap();
        assert_eq!(t.next_action(now,2048).unwrap().0,0x51);reply(&mut t,0x51,0,&[],now);
        assert_eq!(t.queued(),0);assert_eq!(t.next_action(now,2048).unwrap().0,0x52);
        t.apply_at(&Response{op:0x51,req:5,status:0,payload:&[]},now).unwrap();assert_eq!(t.queued(),0);
    }
    #[test] fn open_busy_waits_and_recovers_without_losing_the_request(){
        let now=Instant::now();let mut t=Terminal::default();t.begin_open(now);
        assert_eq!(t.next_action(now,2048).unwrap().0,0x50);reply(&mut t,0x50,8,&[],now);
        assert!(t.starting());assert!(t.needs_timer());assert!(t.message.contains("回收"));
        assert!(t.next_action(now+Duration::from_millis(499),2048).is_none());
        assert_eq!(t.next_action(now+RETRY_DELAY,2048).unwrap().0,0x50);
        reply(&mut t,0x50,0,&[9,0,0,0,2,0,0,0,0],now+RETRY_DELAY);
        assert!(!t.starting());assert_eq!(t.lease,9);assert!(t.can_send());
    }
    #[test] fn repeated_busy_cannot_extend_open_deadline(){
        let now=Instant::now();let mut t=Terminal::default();t.begin_open(now);
        for i in 0..30{let at=now+Duration::from_millis(i*500);assert_eq!(t.next_action(at,2048).unwrap().0,0x50);reply(&mut t,0x50,8,&[],at);}
        assert!(t.next_action(now+OPEN_WAIT,2048).is_none());assert!(!t.starting());assert!(!t.active());assert!(t.message.contains("尚未回收"));
    }
    #[test] fn connection_denied_is_not_retried_as_busy(){
        let now=Instant::now();let mut t=Terminal::default();t.begin_open(now);t.pending=Some((5,0x50,now));
        assert!(t.apply_at(&Response{op:0x50,req:5,status:3,payload:&[]},now).is_err());
        assert!(!t.starting());assert!(t.next_action(now+RETRY_DELAY,2048).is_none());
    }
    #[test] fn cancelled_open_that_completes_late_is_closed_instead_of_reused(){
        let now=Instant::now();let mut t=Terminal::default();t.begin_open(now);t.pending=Some((5,0x50,now));t.stop();
        t.apply_at(&Response{op:0x50,req:5,status:0,payload:&[9,0,0,0,2,0,0,0,0]},now).unwrap();
        assert_eq!(t.next_action(now,2048).unwrap(),(0x53,9u32.to_le_bytes().to_vec()));assert!(!t.can_send());
    }
    #[test] fn rejected_busy_write_retains_payload_and_retries_only_after_backoff(){
        let now=Instant::now();let mut t=running();t.input="echo only-once".into();t.enqueue(false,2048).unwrap();
        let expected=t.next_action(now,2048).unwrap();reply(&mut t,0x51,8,&[],now);
        assert_eq!(t.queued(),1);assert!(t.active());
        let next=t.next_action(now+Duration::from_millis(100),2048).unwrap();assert_eq!(next.0,0x52);
        assert_eq!(t.next_action(now+RETRY_DELAY,2048).unwrap(),expected);
        reply(&mut t,0x51,0,&[],now+RETRY_DELAY);assert_eq!(t.queued(),0);
    }
    #[test] fn transport_failure_never_replays_a_potentially_executed_command(){
        let now=Instant::now();let mut t=running();t.input="reboot".into();t.enqueue(false,2048).unwrap();
        t.next_action(now,2048);t.send_failed("socket closed");
        assert_eq!(t.queued(),0);assert!(t.next_action(now+RETRY_DELAY,2048).is_none());assert!(!t.active());
    }
    #[test] fn prolonged_write_busy_expires_without_replaying_after_deadline(){
        let now=Instant::now();let mut t=running();t.input="pwd".into();t.enqueue(false,2048).unwrap();reply(&mut t,0x51,8,&[],now);
        let action=t.next_action(now+WRITE_WAIT,2048).unwrap();assert_eq!(action.0,0x52);assert_eq!(t.queued(),0);assert!(t.active());assert!(t.message.contains("未被接收"));
    }
    #[test] fn native_editor_defers_render_until_dialog_finishes_and_keeps_draft(){
        let now=Instant::now();let mut t=running();t.editing=true;
        assert!(t.defer_render(now+Duration::from_secs(30)));
        t.edit("echo \"中文\\path\"".into(),now);t.editing=false;
        assert_eq!(t.input,"echo \"中文\\path\"");assert!(!t.defer_render(now));
    }
    #[test] fn cancelling_native_editor_retains_unsent_draft(){
        let now=Instant::now();let mut t=running();t.input="echo keep".into();t.editing=true;t.finish_edit(None,now);
        assert_eq!(t.input,"echo keep");assert!(!t.editing);assert_eq!(t.queued(),0);
        t.editing=true;t.finish_edit(Some("pwd".into()),now);assert_eq!(t.input,"pwd");assert!(!t.editing);
    }
    #[test] fn input_validation_preserves_draft_and_queue_is_bounded(){
        let now=Instant::now();let mut t=running();t.edit("你好".into(),now);
        assert!(t.enqueue(false,6).is_err());assert_eq!(t.input,"你好");assert_eq!(t.queued(),0);
        for _ in 0..MAX_COMMANDS{t.input="pwd".into();t.enqueue(false,2048).unwrap();}
        assert!(!t.can_send());t.input="keep me".into();assert!(t.enqueue(false,2048).is_err());assert_eq!(t.input,"keep me");
    }
    #[test] fn control_c_does_not_reorder_the_in_flight_write(){
        let now=Instant::now();let mut t=running();t.input="ls".into();t.enqueue(false,2048).unwrap();t.pending=Some((5,0x51,now));
        t.enqueue(true,2048).unwrap();reply(&mut t,0x51,0,&[],now);
        assert_eq!(t.next_action(now,2048).unwrap().1,[7u32.to_le_bytes().as_slice(),&[3]].concat());
    }
    #[test] fn split_utf8_ansi_and_bounded_history(){let mut t=Terminal::default();t.append(&[0xe4,0xbd]);t.append(&[0xa0]);t.append(b"\x1b[31mhi\x1b[0m\x1b]title\x07\n");assert_eq!(t.output,"你hi\n");t.append("字".repeat(20000).as_bytes());assert!(t.output.len()<=32768);}
    #[test] fn ignores_old_request_and_rejects_wrong_lease(){let now=Instant::now();let mut t=running();t.pending=Some((5,0x52,now));assert!(t.apply_at(&Response{op:0x52,req:4,status:0,payload:&[99,0,0,0,2,0,0,0,0]},now).is_ok());assert_eq!(t.lease,7);assert!(t.busy());assert!(t.apply_at(&Response{op:0x52,req:5,status:0,payload:&[99,0,0,0,2,0,0,0,0]},now).is_err());}
    #[test] fn connection_revocation_stops_console(){let now=Instant::now();let mut t=running();t.pending=Some((3,0x52,now));assert!(t.apply_at(&Response{op:0x52,req:3,status:3,payload:&[]},now).is_err());assert_eq!(t.lease,0);assert!(!t.can_send());}
}
