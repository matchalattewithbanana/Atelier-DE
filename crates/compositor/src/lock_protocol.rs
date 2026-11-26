//! Own the exact lock and role resources rather than relying on Smithay 0.7's
//! private output bookkeeping. The global is deliberately NOT registered.
use super::Atelier;
use smithay::{
    backend::renderer::utils::RendererSurfaceStateUserData,
    input::{keyboard::FilterResult,pointer::{ButtonEvent,CursorImageStatus,MotionEvent}},
    backend::input::{KeyState,ButtonState},
    utils::SERIAL_COUNTER,
    wayland::{
        compositor::{self,with_states,SurfaceAttributes,BufferAssignment},
        selection::data_device::{set_data_device_focus,clear_data_device_selection},
    },
    reexports::{
        wayland_protocols::ext::session_lock::v1::server::{
            ext_session_lock_manager_v1::{self,ExtSessionLockManagerV1},
            ext_session_lock_v1::{self,ExtSessionLockV1},
            ext_session_lock_surface_v1::{self,ExtSessionLockSurfaceV1},
        },
        wayland_server::{backend::{ClientId,ObjectId},protocol::wl_surface::WlSurface,
            Client,DataInit,Dispatch,DisplayHandle,GlobalDispatch,New,Resource},
    },
};
use crate::session_lock::ProtectedFrame;

const ROLE:&str="ext_session_lock_surface_v1";

#[derive(Debug)]
struct Configure{
    serial:u32,
    size:(i32,i32),
    acked:bool,
}
impl Configure{
    fn ack(&mut self,serial:u32)->bool{
        if self.acked||serial!=self.serial{return false;}
        self.acked=true;true
    }
    fn matches(&self,size:Option<(i32,i32)>)->bool{self.acked&&size==Some(self.size)}
}
struct AcceptedSurface{
    role:ExtSessionLockSurfaceV1,
    root:WlSurface,
    configure:Configure,
    committed:bool,
    invalid:bool,
}
#[derive(Default)]
pub(super) struct Protocol{
    owner:Option<ExtSessionLockV1>,
    surface:Option<AcceptedSurface>,
    recovery_client:Option<ClientId>,
    recovering:bool,
}

impl Atelier{
    pub(super) fn confirm_lock_frame(&mut self,frame:ProtectedFrame){
        if self.session_lock.protected_frame()!=Some(frame){return;}
        let Some(owner)=self.lock_protocol.owner.clone()else{return;};
        if !owner.is_alive(){self.lock_owner_lost(&owner.id());return;}
        if self.session_lock.presented(frame){self.lock_protocol.recovering=false;owner.locked();self.scene_dirty=true;}
    }
    // Lock admission refuses existing grabs; at the other boundary only
    // lock-era click grabs can remain (data-device start_drag is guarded).
    // Clear both seat focuses before changing ownership or restoring desktop.
    fn isolate_input(&mut self,time:u32){
        self.input_gate.boundary();
        let serial=SERIAL_COUNTER.next_serial();
        if let Some(keyboard)=self.seat.get_keyboard(){
            if keyboard.is_grabbed(){keyboard.unset_grab(self);}
            keyboard.set_focus(self,None,serial);
        }
        if let Some(pointer)=self.seat.get_pointer(){
            let location=self.pointer_position;
            let event=MotionEvent{location,serial,time};
            // Update pending focus first; Smithay unset_grab restores it.
            pointer.motion(self,None,&event);
            if pointer.is_grabbed(){pointer.unset_grab(self,serial,time);}
            pointer.motion(self,None,&event);pointer.frame(self);
        }
        set_data_device_focus(&self.display,&self.seat,None);
        self.cursor_status=CursorImageStatus::default_named();
        self.scene_dirty=true;
    }
    /// Revoke the old lock authority and neutralize logical seat state
    /// before entering hardware quarantine. No focus can be restored here.
    pub(crate) fn prepare_protected_recovery(&mut self){
        if self.desktop_visible(){return;}
        if let Some(owner)=self.lock_protocol.owner.as_ref(){self.session_lock.disconnected(&owner.id());}
        self.lock_protocol=Protocol::default();
        self.isolate_input(0);
        let (keys,buttons)=self.input_gate.reset_after_loss();
        let serial=SERIAL_COUNTER.next_serial();
        if let Some(keyboard)=self.seat.get_keyboard(){
            for key in keys{
                keyboard.input(self,key,KeyState::Released,serial,0,|_,_,_|FilterResult::<()>::Forward);
            }
        }
        if let Some(pointer)=self.seat.get_pointer(){
            for button in buttons{
                pointer.button(self,&ButtonEvent{button,state:ButtonState::Released,serial,time:0});
            }
            if pointer.is_grabbed(){pointer.unset_grab(self,serial,0);}
            pointer.frame(self);
        }
        clear_data_device_selection(&self.display,&self.seat);
        self.scene_dirty=true;
    }
    /// A trusted internal supervisor may pass this private socket endpoint
    /// directly to its replacement locker. Never expose it via workspace IPC,
    /// a filename, PID/UID matching or an untrusted client request.
    /// No production caller exists until hardware recovery and locker launch
    /// are validated. Authentication remains the replacement locker's job.
    pub(crate) fn recovery_connection(&mut self)->anyhow::Result<(ClientId,std::os::unix::net::UnixStream)>{
        if !self.session_lock.orphaned()||self.lock_protocol.recovery_client.is_some(){
            anyhow::bail!("recovery requires an orphaned lock without an outstanding grant");
        }
        let (server,endpoint)=std::os::unix::net::UnixStream::pair()?;
        let client=self.display.insert_client(server,std::sync::Arc::new(super::ClientState::default()))?;
        let id=client.id();
        self.lock_protocol.recovery_client=Some(id.clone());
        Ok((id,endpoint))
    }
    /// Scoped cancellation: do not revoke a later grant or unrelated owner.
    pub(crate) fn revoke_recovery(&mut self,id:&ClientId){
        if self.lock_protocol.recovery_client.as_ref()==Some(id){self.lock_protocol.recovery_client=None;}
        let owns=self.lock_protocol.owner.as_ref().and_then(|owner|owner.client()).is_some_and(|client|&client.id()==id);
        if owns{self.prepare_protected_recovery();}
    }
    pub(crate) fn recovery_confirmed(&self,id:&ClientId)->bool{
        self.lock_protocol.owner.as_ref().is_some_and(|owner|owner.client().is_some_and(|client|&client.id()==id)&&self.session_lock.can_unlock(&owner.id()))
    }
    pub(super) fn recovery_input_ready(&self)->bool{!self.lock_protocol.recovering}
    pub(super) fn sync_lock_input(&mut self,time:u32){
        if !self.recovery_input_ready(){return;}
        if self.desktop_visible(){
            if self.restore_input&&!self.input_gate.waiting(){
                self.restore_input=false;
                let serial=SERIAL_COUNTER.next_serial();
                let window=self.focused.clone().filter(|w|self.space.element_location(w).is_some())
                    .or_else(||self.space.elements().next_back().cloned());
                self.focus(window,serial);
                if let Some(pointer)=self.seat.get_pointer(){
                    let focus=self.surface_under_pointer();let location=self.pointer_position;
                    pointer.motion(self,focus,&MotionEvent{location,serial,time});pointer.frame(self);
                }
            }
            return;
        }
        let target=self.lock_surface().cloned();
        let keyboard=self.seat.get_keyboard();
        let current=keyboard.as_ref().and_then(|k|k.current_focus());
        // A newly mapped or vanished lock root creates another boundary.
        // Do not leak held keys from a previously focused root into a new one.
        if current.is_some()&&current!=target{
            self.isolate_input(time);
        }
        if self.input_gate.waiting(){return;}
        if let Some(keyboard)=keyboard{
            if keyboard.current_focus()!=target{
                // A key may have been pressed before any lock surface existed.
                // Require it to be released before entering the new focus.
                self.input_gate.boundary();
                if self.input_gate.waiting(){return;}
                keyboard.set_focus(self,target.clone(),SERIAL_COUNTER.next_serial());
            }
        }
        // Refresh lock pointer focus on entry or on surface changes; ordinary
        // motion uses the same lock-only hit-test. Respect lock click grabs.
        if let Some(pointer)=self.seat.get_pointer(){
            if !pointer.is_grabbed(){
                let focus=self.input_pointer_focus();
                if pointer.current_focus()!=focus.as_ref().map(|(surface,_)|surface.clone()){
                    let location=self.pointer_position;
                    pointer.motion(self,focus,&MotionEvent{location,serial:SERIAL_COUNTER.next_serial(),time});
                    pointer.frame(self);
                }
            }
        }
    }

    fn clear_role(&mut self,id:&ObjectId){
        if self.lock_protocol.surface.as_ref().is_some_and(|s|s.role.id()==*id){
            self.lock_protocol.surface=None;
            self.isolate_input(0);
        }
    }
    pub(super) fn lock_root_destroyed(&mut self,root:&WlSurface){
        if let Some(id)=self.lock_protocol.surface.as_ref().filter(|s|s.root==*root).map(|s|s.role.id()){
            self.clear_role(&id);
        }
    }
    fn lock_owner_lost(&mut self,id:&ObjectId){
        if self.session_lock.disconnected(id){
            self.lock_protocol=Protocol::default();
            self.isolate_input(0);
        }
    }
    fn buffer_size(root:&WlSurface)->Option<(i32,i32)>{
        with_states(root,|states|{
            let state=states.data_map.get::<RendererSurfaceStateUserData>()?.lock().ok()?;
            state.buffer()?;
            let size=state.surface_size()?;
            Some((size.w,size.h))
        })
    }
    // Validate the request at submission time as well as after application:
    // a delayed surface transaction must not be legitimized by a later ack.
    fn validate_lock_commit(&mut self,root:&WlSurface){
        use ext_session_lock_surface_v1::Error;
        let Some(s)=self.lock_protocol.surface.as_mut().filter(|s|s.root==*root)else{return;};
        if s.invalid{return;}
        let removed=with_states(root,|states|{
            matches!(states.cached_state.get::<SurfaceAttributes>().pending().buffer,Some(BufferAssignment::Removed))
        });
        let error=if !s.configure.acked{Some(Error::CommitBeforeFirstAck)}
            else if removed{Some(Error::NullBuffer)}else{None};
        if let Some(error)=error{
            s.invalid=true;s.role.post_error(error,"invalid lock surface commit submission");
            self.scene_dirty=true;
        }
    }
    // Called after Smithay processes the root commit, before any render or
    // input dispatch. A failed commit removes the accepted role immediately.
    pub(super) fn lock_root_committed(&mut self,root:&WlSurface){
        use ext_session_lock_surface_v1::Error;
        let Some(s)=self.lock_protocol.surface.as_mut().filter(|s|s.root==*root)else{return;};
        if s.invalid{let id=s.role.id();self.clear_role(&id);return;}
        let size=Self::buffer_size(root);
        let error=if !s.configure.acked{Some(Error::CommitBeforeFirstAck)}
            else if size.is_none(){Some(Error::NullBuffer)}
            else if !s.configure.matches(size){Some(Error::DimensionsMismatch)}
            else{None};
        if let Some(error)=error{
            s.role.post_error(error,"invalid lock surface commit");
            let id=s.role.id();self.clear_role(&id);
        }else{s.committed=true;}
    }
    pub(crate) fn lock_surface(&self)->Option<&WlSurface>{
        let owner=self.lock_protocol.owner.as_ref()?;
        if self.desktop_visible()||!owner.is_alive()||!self.session_lock.owns_request(&owner.id()){return None;}
        let s=self.lock_protocol.surface.as_ref()?;
        if !s.role.is_alive()||!s.root.is_alive()||!s.committed||s.invalid{return None;}
        let mode=self.output.current_mode()?;
        if s.configure.size!=(mode.size.w,mode.size.h)||!s.configure.matches(Self::buffer_size(&s.root)){return None;}
        Some(&s.root)
    }
    fn accept_lock(&mut self,owner:ExtSessionLockV1){
        // A new ordinary client is never recovery authority. Only the exact
        // server-created private connection may replace an orphaned owner.
        if self.session_lock.orphaned(){
            let granted=owner.client().is_some_and(|c|self.lock_protocol.recovery_client.as_ref()==Some(&c.id()));
            if !granted||!self.session_lock.recover(owner.id()){owner.finished();return;}
            self.lock_protocol.recovery_client=None;
            self.lock_protocol.recovering=true;
            self.lock_protocol.owner=Some(owner);
            self.isolate_input(0);
            clear_data_device_selection(&self.display,&self.seat);
            return;
        }
        if !self.desktop_input_ready()||self.seat.get_pointer().is_some_and(|p|p.is_grabbed())||
            self.seat.get_keyboard().is_some_and(|k|k.is_grabbed())||!self.session_lock.begin(owner.id()){
            owner.finished();return;
        }
        self.lock_protocol.owner=Some(owner);
        self.isolate_input(0);
        clear_data_device_selection(&self.display,&self.seat);
    }
    fn unlock_owner(&mut self,owner:&ExtSessionLockV1){
        if !self.session_lock.can_unlock(&owner.id()){
            owner.post_error(ext_session_lock_v1::Error::InvalidUnlock,"only the confirmed lock object may unlock");return;
        }
        self.isolate_input(0);
        self.restore_input=true;
        if self.session_lock.unlock(&owner.id()){
            self.lock_protocol=Protocol::default();
            self.scene_dirty=true;
        }
    }
}

// These implementations intentionally have no create_global call in any
// production or nested backend.
impl GlobalDispatch<ExtSessionLockManagerV1,()> for Atelier{
    fn bind(_state:&mut Self,_dh:&DisplayHandle,_client:&Client,manager:New<ExtSessionLockManagerV1>,_data:&(),init:&mut DataInit<'_,Self>){
        init.init(manager,());
    }
}
impl Dispatch<ExtSessionLockManagerV1,()> for Atelier{
    fn request(state:&mut Self,_client:&Client,_manager:&ExtSessionLockManagerV1,request:ext_session_lock_manager_v1::Request,
        _data:&(),_dh:&DisplayHandle,init:&mut DataInit<'_,Self>){
        if let ext_session_lock_manager_v1::Request::Lock{id}=request{
            state.accept_lock(init.init(id,()));
        }
    }
}
impl Dispatch<ExtSessionLockV1,()> for Atelier{
    fn request(state:&mut Self,_client:&Client,lock:&ExtSessionLockV1,request:ext_session_lock_v1::Request,
        _data:&(),_dh:&DisplayHandle,init:&mut DataInit<'_,Self>){
        use ext_session_lock_v1::{Error,Request};
        match request{
            Request::UnlockAndDestroy=>state.unlock_owner(lock),
            Request::Destroy=>{
                if state.session_lock.can_unlock(&lock.id()){
                    lock.post_error(Error::InvalidDestroy,"confirmed lock requires unlock_and_destroy");
                }
                state.lock_owner_lost(&lock.id());
            }
            Request::GetLockSurface{id,surface,output}=>{
                if !state.session_lock.owns_request(&lock.id()){
                    lock.post_error(Error::InvalidUnlock,"rejected lock cannot create lock surfaces");return;
                }
                if state.lock_protocol.surface.is_some()||!state.output.owns(&output){
                    lock.post_error(Error::DuplicateOutput,"only one role on the active output is supported");return;
                }
                let Some(mode)=state.output.current_mode().filter(|m|m.size.w>0&&m.size.h>0)else{
                    lock.post_error(Error::DuplicateOutput,"no usable output");return;
                };
                if compositor::get_role(&surface).is_some(){
                    lock.post_error(Error::Role,"surface already has a role");return;
                }
                let has_buffer=with_states(&surface,|states|{
                    let mut attrs=states.cached_state.get::<SurfaceAttributes>();
                    matches!(attrs.pending().buffer,Some(BufferAssignment::NewBuffer(_)))||
                        matches!(attrs.current().buffer,Some(BufferAssignment::NewBuffer(_)))||
                        states.data_map.get::<RendererSurfaceStateUserData>()
                            .is_some_and(|s|s.lock().map_or(true,|s|s.buffer().is_some()))
                });
                if has_buffer{lock.post_error(Error::AlreadyConstructed,"surface already has a buffer");return;}
                if compositor::give_role(&surface,ROLE).is_err(){lock.post_error(Error::Role,"surface already has a role");return;}
                let role=init.init(id,());
                let serial:u32=SERIAL_COUNTER.next_serial().into();
                let size=(mode.size.w,mode.size.h);
                role.configure(serial,size.0 as u32,size.1 as u32);
                compositor::add_pre_commit_hook::<Atelier,_>(&surface,|state,_,root|state.validate_lock_commit(root));
                state.lock_protocol.surface=Some(AcceptedSurface{role,root:surface,configure:Configure{serial,size,acked:false},committed:false,invalid:false});
                state.scene_dirty=true;
            }
            _=>{}
        }
    }
    fn destroyed(state:&mut Self,_client:ClientId,lock:&ExtSessionLockV1,_data:&()){
        state.lock_owner_lost(&lock.id());
    }
}
impl Dispatch<ExtSessionLockSurfaceV1,()> for Atelier{
    fn request(state:&mut Self,_client:&Client,role:&ExtSessionLockSurfaceV1,request:ext_session_lock_surface_v1::Request,
        _data:&(),_dh:&DisplayHandle,_init:&mut DataInit<'_,Self>){
        use ext_session_lock_surface_v1::{Error,Request};
        match request{
            Request::Destroy=>state.clear_role(&role.id()),
            Request::AckConfigure{serial}=>{
                let Some(s)=state.lock_protocol.surface.as_mut().filter(|s|s.role.id()==role.id())else{return;};
                if s.invalid{return;}
                if !s.configure.ack(serial){
                    role.post_error(Error::InvalidSerial,"unknown or repeated configure serial");
                    state.clear_role(&role.id());
                }
            }
            _=>{}
        }
    }
    fn destroyed(state:&mut Self,_client:ClientId,role:&ExtSessionLockSurfaceV1,_data:&()){
        state.clear_role(&role.id());
    }
}

#[cfg(test)]
#[path = "lock_wire_tests.rs"]
mod wire_tests;

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn configure_requires_exact_serial_and_dimensions(){
        let mut c=Configure{serial:12,size:(1920,1080),acked:false};
        assert!(!c.matches(Some((1920,1080))));
        assert!(!c.ack(11));assert!(!c.acked);
        assert!(c.ack(12));assert!(!c.ack(12));
        assert!(!c.matches(None));assert!(!c.matches(Some((1920,1079))));
        assert!(c.matches(Some((1920,1080))));
    }
}
