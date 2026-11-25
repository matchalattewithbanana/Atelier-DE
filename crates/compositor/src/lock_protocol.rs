//! Native session-lock adapter for Smithay 0.7. Deliberately unadvertised.
//! The upstream resource handler must not see foreign or premature unlocks:
//! it calls unlock even after posting InvalidUnlock in this pinned version.
use super::Atelier;
use smithay::{
    backend::renderer::utils::RendererSurfaceStateUserData,
    reexports::{
        wayland_protocols::ext::session_lock::v1::server::{
            ext_session_lock_manager_v1::ExtSessionLockManagerV1,
            ext_session_lock_v1::{self,ExtSessionLockV1},
            ext_session_lock_surface_v1::{self,ExtSessionLockSurfaceV1},
        },
        wayland_server::{backend::{ClientId,ObjectId},protocol::{wl_output::WlOutput,wl_surface::WlSurface},
            Client,DataInit,Dispatch,DisplayHandle,Resource},
    },
    wayland::{compositor::with_states,session_lock::{ExtLockSurfaceUserData,LockSurface,SessionLockHandler,
        SessionLockManagerGlobalData,SessionLockManagerState,SessionLockState,SessionLocker}},
};
use crate::session_lock::ProtectedFrame;

#[derive(Default)]
pub(super) struct Protocol{
    // None for every shipped backend. No code currently creates the global.
    manager:Option<SessionLockManagerState>,
    pending:Option<SessionLocker>,
    owner:Option<ObjectId>,
    surface:Option<LockSurface>,
    surface_issued:bool,
    authorized_unlock:Option<ObjectId>,
}

// Delegate global and manager creation, but NOT lock resource requests.
smithay::reexports::wayland_server::delegate_global_dispatch!(Atelier: [
    ExtSessionLockManagerV1: SessionLockManagerGlobalData
] => SessionLockManagerState);
smithay::reexports::wayland_server::delegate_dispatch!(Atelier: [
    ExtSessionLockManagerV1: ()
] => SessionLockManagerState);

impl Atelier{
    pub(super) fn confirm_lock_frame(&mut self,frame:ProtectedFrame){
        if self.session_lock.protected_frame()!=Some(frame){return;}
        let Some(pending)=self.lock_protocol.pending.take()else{return;};
        let id=pending.ext_session_lock().id();
        if !pending.ext_session_lock().is_alive(){
            self.lock_owner_lost(&id);return;
        }
        if self.session_lock.presented(frame){
            pending.lock();
            self.scene_dirty=true;
        }
    }
    fn clear_lock_surface_for_client(&mut self,client:&ClientId){
        let accepted=self.lock_protocol.surface.as_ref().and_then(|s|s.wl_surface().client());
        if accepted.as_ref().is_some_and(|c|&c.id()==client){
            self.lock_protocol.surface=None;
            self.scene_dirty=true;
        }
    }
    fn lock_owner_lost(&mut self,id:&ObjectId){
        if self.session_lock.disconnected(id){
            self.lock_protocol.pending=None;
            self.lock_protocol.owner=None;
            self.lock_protocol.surface=None;
            self.lock_protocol.authorized_unlock=None;
            self.scene_dirty=true;
        }
    }
    /// Direct surface only, not a desktop window. The accepted root must
    /// be alive and have a committed output-sized buffer.
    pub(crate) fn lock_surface(&self)->Option<&WlSurface>{
        if self.desktop_visible(){return None;}
        let owner=self.lock_protocol.owner.as_ref()?;
        if !self.session_lock.owns_request(owner){return None;}
        let surface=self.lock_protocol.surface.as_ref()?;
        if !surface.alive(){return None;}
        let mode=self.output.current_mode()?;
        let size=surface.current_state().size?;
        if mode.size.w<=0||mode.size.h<=0||size.w!=mode.size.w as u32||size.h!=mode.size.h as u32{return None;}
        let wl=surface.wl_surface();
        let mapped=with_states(wl,|states|{
            states.data_map.get::<RendererSurfaceStateUserData>()
                .is_some_and(|state|state.lock().is_ok_and(|s|s.buffer().is_some()))
        });
        mapped.then_some(wl)
    }
}
impl SessionLockHandler for Atelier{
    fn lock_state(&mut self)->&mut SessionLockManagerState{
        self.lock_protocol.manager.as_mut().expect("session-lock manager must be initialized before dispatch")
    }
    fn lock(&mut self,confirmation:SessionLocker){
        let id=confirmation.ext_session_lock().id();
        if !self.session_lock.begin(id.clone()){return;}
        self.lock_protocol.owner=Some(id);
        self.lock_protocol.surface_issued=false;
        self.lock_protocol.pending=Some(confirmation);
        self.scene_dirty=true;
    }
    fn unlock(&mut self){
        // Only the guarded resource dispatch can arm this one-shot context.
        let Some(id)=self.lock_protocol.authorized_unlock.take()else{return;};
        if self.session_lock.unlock(&id){
            self.lock_protocol.owner=None;
            self.lock_protocol.surface=None;
            self.lock_protocol.pending=None;
            self.scene_dirty=true;
        }
    }
    fn new_surface(&mut self,surface:LockSurface,output:WlOutput){
        // GetLockSurface dispatch has already authorized the object and
        // physical output. The single output is currently unscaled.
        if !self.output.owns(&output)||self.lock_protocol.surface.is_some(){return;}
        let Some(mode)=self.output.current_mode()else{return;};
        if mode.size.w<=0||mode.size.h<=0{return;}
        surface.with_pending_state(|s|s.size=Some((mode.size.w as u32,mode.size.h as u32).into()));
        self.lock_protocol.surface=Some(surface);
        self.lock_protocol.surface_issued=true;
        self.scene_dirty=true;
    }
}

impl Dispatch<ExtSessionLockV1,SessionLockState> for Atelier{
    fn request(state:&mut Self,client:&Client,lock:&ExtSessionLockV1,request:ext_session_lock_v1::Request,
        data:&SessionLockState,display:&DisplayHandle,init:&mut DataInit<'_,Self>){
        use ext_session_lock_v1::{Error,Request};
        let owner=lock.id();
        match &request{
            Request::UnlockAndDestroy=>{
                if !state.session_lock.can_unlock(&owner){
                    lock.post_error(Error::InvalidUnlock,"only the confirmed lock object may unlock");
                    return;
                }
                state.lock_protocol.authorized_unlock=Some(owner);
            }
            Request::GetLockSurface{output,..}=>{
                if !state.session_lock.owns_request(&owner){
                    // Follow the upstream fatal-error pattern for a rejected
                    // constructor, without touching its shared output registry.
                    lock.post_error(Error::InvalidUnlock,"rejected lock cannot create lock surfaces");
                    return;
                }
                if state.lock_protocol.surface_issued||!state.output.owns(output)||
                    !state.output.current_mode().is_some_and(|m|m.size.w>0&&m.size.h>0){
                    lock.post_error(Error::DuplicateOutput,"only one lock surface on the active output is supported");
                    return;
                }
            }
            Request::Destroy=>{
                if state.session_lock.can_unlock(&owner){
                    lock.post_error(Error::InvalidDestroy,"confirmed lock requires unlock_and_destroy");
                }
                // No upstream mutation for rejected or abandoned locks.
                state.lock_owner_lost(&owner);
                return;
            }
            _=>return,
        }
        <SessionLockManagerState as Dispatch<ExtSessionLockV1,SessionLockState,Self>>::request(
            state,client,lock,request,data,display,init);
        state.lock_protocol.authorized_unlock=None;
    }
    fn destroyed(state:&mut Self,_client:ClientId,lock:&ExtSessionLockV1,_data:&SessionLockState){
        state.lock_owner_lost(&lock.id());
    }
}

impl Dispatch<ExtSessionLockSurfaceV1,ExtLockSurfaceUserData> for Atelier{
    fn request(state:&mut Self,client:&Client,role:&ExtSessionLockSurfaceV1,request:ext_session_lock_surface_v1::Request,
        data:&ExtLockSurfaceUserData,display:&DisplayHandle,init:&mut DataInit<'_,Self>){
        // The pinned Smithay role userdata is private. On any destroy by
        // the accepted surface's client, conservatively drop the entire
        // accepted tree, rather than risk retaining a destroyed role.
        // Stale same-client role destruction may blank the lock UI; a
        // precise resource association and recovery remain release gates.
        if matches!(&request,ext_session_lock_surface_v1::Request::Destroy){
            state.clear_lock_surface_for_client(&client.id());
        }
        <SessionLockManagerState as Dispatch<ExtSessionLockSurfaceV1,ExtLockSurfaceUserData,Self>>::request(
            state,client,role,request,data,display,init);
    }
    fn destroyed(state:&mut Self,client:ClientId,_role:&ExtSessionLockSurfaceV1,_data:&ExtLockSurfaceUserData){
        state.clear_lock_surface_for_client(&client);
    }
}
