//! In-process Wayland wire tests, over private socket pairs only. No actual
//! compositor socket, GPU, seat, authentication, or global in shipped code.
//! Encode only the few core and ext-session-lock v1 messages used here.
use super::*;
use crate::state::ClientState;
use smithay::reexports::wayland_server::Display;
use std::{collections::BTreeMap,io::{self,Read,Write},os::unix::net::UnixStream,sync::Arc,
    time::{Duration,Instant}};

struct Harness{
    display:Display<Atelier>,
    state:Atelier,
}
impl Harness{
    fn new()->Self{
        let display=Display::new().unwrap();
        let mut state=Atelier::new_for_output(&display.handle(),Default::default(),"LOCK-TEST-1","Private test output").unwrap();
        state.resize(64,48);
        // Only this cfg(test) module can register the experimental global.
        display.handle().create_global::<Atelier,ExtSessionLockManagerV1,_>(1,());
        Self{display,state}
    }
    fn client(&mut self)->Wire{
        let (server,client)=UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        self.display.handle().insert_client(server,Arc::new(ClientState::default())).unwrap();
        let mut wire=Wire{socket:client,next:2,bytes:vec![],events:vec![],closed:false,registry:None};
        let registry=wire.id();wire.words(1,1,&[registry]);wire.roundtrip(self);
        let mut globals=BTreeMap::new();
        for event in wire.take(){
            if event.object==registry&&event.opcode==0{
                let name=word(&event.payload,0);
                let len=word(&event.payload,4) as usize;
                let end=8+len;
                let interface=std::str::from_utf8(&event.payload[8..end-1]).unwrap().to_owned();
                let version=word(&event.payload,(end+3)&!3);
                globals.insert(interface,(name,version));
            }
        }
        for (interface,version) in [("ext_session_lock_manager_v1",1),("wl_compositor",4),("wl_output",1)]{
            assert!(globals[interface].1>=version);
        }
        wire.registry=Some((registry,globals));
        wire
    }
    fn step(&mut self){
        self.display.dispatch_clients(&mut self.state).unwrap();
        self.display.flush_clients().unwrap();
    }
    fn confirm(&mut self){
        let token=self.state.protected_frame().expect("pending protected frame");
        self.state.protected_frame_presented(token);
    }
}

#[derive(Debug)]
struct Event{object:u32,opcode:u16,payload:Vec<u8>}
fn word(bytes:&[u8],at:usize)->u32{u32::from_ne_bytes(bytes[at..at+4].try_into().unwrap())}
fn push(bytes:&mut Vec<u8>,value:u32){bytes.extend_from_slice(&value.to_ne_bytes());}
struct Wire{
    socket:UnixStream,next:u32,bytes:Vec<u8>,events:Vec<Event>,closed:bool,
    registry:Option<(u32,BTreeMap<String,(u32,u32)>)>,
}
impl Wire{
    fn id(&mut self)->u32{let id=self.next;self.next+=1;id}
    fn send(&mut self,object:u32,opcode:u16,payload:&[u8]){
        assert_eq!(payload.len()%4,0);
        let mut message=Vec::new();push(&mut message,object);
        push(&mut message,(((payload.len()+8) as u32)<<16)|u32::from(opcode));
        message.extend_from_slice(payload);
        self.socket.write_all(&message).unwrap();
    }
    fn words(&mut self,object:u32,opcode:u16,words:&[u32]){
        let mut payload=vec![];for &w in words{push(&mut payload,w);}
        self.send(object,opcode,&payload);
    }
    fn bind(&mut self,interface:&str,version:u32)->u32{
        let id=self.id();let (registry,globals)=self.registry.as_ref().unwrap();
        let name=globals[interface].0;let registry=*registry;
        let mut payload=vec![];push(&mut payload,name);push(&mut payload,(interface.len()+1) as u32);
        payload.extend_from_slice(interface.as_bytes());payload.push(0);
        while payload.len()%4!=0{payload.push(0);}
        push(&mut payload,version);push(&mut payload,id);self.send(registry,0,&payload);id
    }
    fn lock(&mut self,manager:u32)->u32{let id=self.id();self.words(manager,1,&[id]);id}
    fn surface(&mut self,compositor:u32)->u32{let id=self.id();self.words(compositor,0,&[id]);id}
    fn role(&mut self,lock:u32,root:u32,output:u32)->u32{let id=self.id();self.words(lock,1,&[id,root,output]);id}
    fn read(&mut self){
        let mut buf=[0u8;8192];
        loop{
            match self.socket.read(&mut buf){
                Ok(0)=>{self.closed=true;break;}
                Ok(n)=>self.bytes.extend_from_slice(&buf[..n]),
                Err(e) if e.kind()==io::ErrorKind::WouldBlock=>break,
                Err(e) if e.kind()==io::ErrorKind::ConnectionReset=>{self.closed=true;break;},
                Err(e)=>panic!("test wire read: {e}"),
            }
        }
        while self.bytes.len()>=8{
            let header=word(&self.bytes,4);let size=(header>>16) as usize;
            assert!(size>=8&&size%4==0);
            if self.bytes.len()<size{break;}
            self.events.push(Event{object:word(&self.bytes,0),opcode:header as u16,payload:self.bytes[8..size].to_vec()});
            self.bytes.drain(..size);
        }
    }
    fn wait(&mut self,h:&mut Harness,condition:impl Fn(&[Event])->bool){
        let deadline=Instant::now()+Duration::from_secs(3);
        loop{
            h.step();self.read();
            if condition(&self.events){return;}
            assert!(!self.closed,"client disconnected: {:?}",self.events);
            assert!(Instant::now()<deadline,"timed out waiting for wire events: {:?}",self.events);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn roundtrip(&mut self,h:&mut Harness){
        let callback=self.id();self.words(1,0,&[callback]);
        self.wait(h,|events|events.iter().any(|e|e.object==callback&&e.opcode==0));
        assert!(!self.events.iter().any(|e|e.object==1&&e.opcode==0),"unexpected display error: {:?}",self.events);
    }
    fn error(&mut self,h:&mut Harness,object:u32,code:u32){
        self.wait(h,|events|events.iter().any(|e|e.object==1&&e.opcode==0));
        let e=self.events.iter().find(|e|e.object==1&&e.opcode==0).unwrap();
        assert_eq!(word(&e.payload,0),object);assert_eq!(word(&e.payload,4),code);
    }
    fn take(&mut self)->Vec<Event>{std::mem::take(&mut self.events)}
    fn saw(&self,object:u32,opcode:u16)->bool{self.events.iter().any(|e|e.object==object&&e.opcode==opcode)}
    fn configure(&self,role:u32)->u32{
        let e=self.events.iter().find(|e|e.object==role&&e.opcode==0).expect("configure event");
        assert_eq!((word(&e.payload,4),word(&e.payload,8)),(64,48));word(&e.payload,0)
    }
}
fn setup(h:&mut Harness)->(Wire,u32,u32,u32){
    let mut w=h.client();
    let manager=w.bind("ext_session_lock_manager_v1",1);
    let compositor=w.bind("wl_compositor",4);
    let output=w.bind("wl_output",1);
    w.roundtrip(h);w.take();
    (w,manager,compositor,output)
}

#[test]
fn lock_event_waits_for_presented_frame_and_owner_can_unlock(){
    let mut h=Harness::new();let (mut w,m,_,_)=setup(&mut h);
    let lock=w.lock(m);w.roundtrip(&mut h);
    assert!(!h.state.desktop_visible());assert!(!w.saw(lock,0));
    h.confirm();w.roundtrip(&mut h);assert!(w.saw(lock,0));
    w.words(lock,2,&[]);w.roundtrip(&mut h);
    assert!(h.state.desktop_visible());
}
#[test]
fn rejected_same_client_object_cannot_unlock_confirmed_owner(){
    let mut h=Harness::new();let (mut w,m,_,_)=setup(&mut h);
    let owner=w.lock(m);w.roundtrip(&mut h);h.confirm();w.roundtrip(&mut h);
    assert!(w.saw(owner,0));let rejected=w.lock(m);w.roundtrip(&mut h);assert!(w.saw(rejected,1));
    w.words(rejected,2,&[]);w.error(&mut h,rejected,1);
    assert!(!h.state.desktop_visible());
}
#[test]
fn foreign_client_rejection_does_not_destroy_owner(){
    let mut h=Harness::new();let (mut a,m,_,_)=setup(&mut h);
    let owner=a.lock(m);a.roundtrip(&mut h);h.confirm();a.roundtrip(&mut h);
    let (mut b,n,c,o)=setup(&mut h);let rejected=b.lock(n);b.roundtrip(&mut h);
    assert!(b.saw(rejected,1));let root=b.surface(c);let role=b.role(rejected,root,o);
    b.error(&mut h,rejected,1);assert!(!b.saw(role,0));
    assert!(!h.state.desktop_visible());
    a.words(owner,2,&[]);a.roundtrip(&mut h);assert!(h.state.desktop_visible());
}
#[test]
fn exact_surface_destroy_and_replacement_stays_protected(){
    let mut h=Harness::new();let (mut w,m,c,o)=setup(&mut h);
    let lock=w.lock(m);let root=w.surface(c);let role=w.role(lock,root,o);
    w.roundtrip(&mut h);let serial=w.configure(role);
    w.words(role,1,&[serial]);w.roundtrip(&mut h);assert!(h.state.lock_surface().is_none());
    w.words(role,0,&[]);w.roundtrip(&mut h);
    let second=w.surface(c);let new_role=w.role(lock,second,o);w.roundtrip(&mut h);
    assert_ne!(w.configure(new_role),serial);
    // Destruction of the previous root must not clear the replacement.
    w.words(root,0,&[]);w.roundtrip(&mut h);
    assert_eq!(h.state.lock_protocol.surface.as_ref().unwrap().role.id().protocol_id(),new_role);
    h.confirm();w.roundtrip(&mut h);assert!(w.saw(lock,0));assert!(!h.state.desktop_visible());
    w.words(lock,2,&[]);w.roundtrip(&mut h);assert!(h.state.desktop_visible());
}
#[test]
fn invalid_ack_and_commit_remain_protected(){
    for ack in [false,true]{
        let mut h=Harness::new();let (mut w,m,c,o)=setup(&mut h);
        let lock=w.lock(m);let root=w.surface(c);let role=w.role(lock,root,o);w.roundtrip(&mut h);
        let serial=w.configure(role);
        if ack{w.words(role,1,&[serial.wrapping_add(1)]);w.error(&mut h,role,3);}
        else{w.words(root,6,&[]);w.error(&mut h,role,0);}
        assert!(!h.state.desktop_visible());assert!(h.state.lock_surface().is_none());
    }
}
#[test]
fn premature_unlock_and_owner_disconnect_fail_closed(){
    let mut h=Harness::new();let (mut w,m,_,_)=setup(&mut h);
    let lock=w.lock(m);w.roundtrip(&mut h);w.words(lock,2,&[]);w.error(&mut h,lock,1);
    assert!(!h.state.desktop_visible());
    for confirmed in [false,true]{
        let mut h=Harness::new();let (mut w,m,_,_)=setup(&mut h);let _lock=w.lock(m);w.roundtrip(&mut h);
        if confirmed{h.confirm();w.roundtrip(&mut h);}
        drop(w);
        let deadline=Instant::now()+Duration::from_secs(3);
        while h.state.lock_protocol.owner.is_some(){h.step();assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(1));}
        assert!(!h.state.desktop_visible());assert!(h.state.protected_frame().is_none());
        let (mut other,m,_,_)=setup(&mut h);let rejected=other.lock(m);other.roundtrip(&mut h);
        assert!(other.saw(rejected,1));assert!(!h.state.desktop_visible());
    }
}
