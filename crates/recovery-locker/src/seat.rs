//! Seat/keyboard event ownership for the private Wayland recovery connection.
//! The black-only client has no prompt and cannot enable password entry.
use crate::{State,entry::Entry,keyboard::{Keyboard,Intent}};
use std::time::Instant;
use wayland_client::{Connection,Dispatch,QueueHandle,WEnum,protocol::{wl_seat,wl_keyboard,wl_surface}};

#[derive(Default)]
pub(super) struct Input{
    seat:Option<wl_seat::WlSeat>,
    keyboard:Option<wl_keyboard::WlKeyboard>,
    decoder:Keyboard,
    entry:Entry,
}
impl Input{
    pub(super) fn bind(&mut self,seat:wl_seat::WlSeat){self.seat=Some(seat);}
    pub(super) fn revoke(&mut self){
        self.decoder.prompt_ready(&mut self.entry,false);
        self.decoder.remove(&mut self.entry);
    }
    fn capabilities(&mut self,seat:&wl_seat::WlSeat,keyboard:bool,qh:&QueueHandle<State>){
        if self.seat.as_ref()!=Some(seat){return;}
        match (keyboard,self.keyboard.is_some()){
            (true,false)=>self.keyboard=Some(seat.get_keyboard(qh,())),
            (false,true)=>{
                self.revoke();
                if let Some(keyboard)=self.keyboard.take(){keyboard.release();}
            }
            _=>{}
        }
    }
    fn event(&mut self,proxy:&wl_keyboard::WlKeyboard,event:wl_keyboard::Event,root:Option<&wl_surface::WlSurface>)->Result<(),&'static str>{
        // A queued event from a released keyboard cannot restore readiness.
        if self.keyboard.as_ref()!=Some(proxy){return Ok(());}
        match event{
            wl_keyboard::Event::Keymap{format,fd,size}=>{
                if format!=WEnum::Value(wl_keyboard::KeymapFormat::XkbV1){
                    self.revoke();return Err("unsupported keyboard keymap format");
                }
                self.decoder.replace_map(&mut self.entry,fd,size).map_err(|_|"invalid keyboard keymap")?;
            }
            wl_keyboard::Event::Enter{surface,keys,..}=>{
                self.decoder.enter(&mut self.entry,root==Some(&surface),&keys).map_err(|_|"invalid keyboard enter")?;
            }
            wl_keyboard::Event::Leave{..}=>self.decoder.leave(&mut self.entry),
            wl_keyboard::Event::Modifiers{mods_depressed,mods_latched,mods_locked,group,..}=>{
                self.decoder.modifiers(&mut self.entry,mods_depressed,mods_latched,mods_locked,group);
            }
            wl_keyboard::Event::Key{key,state,..}=>{
                let pressed=match state{
                    WEnum::Value(wl_keyboard::KeyState::Pressed)=>true,
                    WEnum::Value(wl_keyboard::KeyState::Released)=>false,
                    _=>{self.revoke();return Err("unsupported keyboard key state");}
                };
                // Even a valid Enter and keymap cannot collect input: no
                // caller in this client marks a graphical prompt as ready.
                if self.decoder.key(&mut self.entry,key,pressed,Instant::now())!=Intent::None{
                    self.revoke();return Err("unexpected password input without prompt");
                }
            }
            _=>{}
        }
        Ok(())
    }
}
impl Dispatch<wl_seat::WlSeat,()> for State{
    fn event(state:&mut Self,seat:&wl_seat::WlSeat,event:wl_seat::Event,_:&(),_:&Connection,qh:&QueueHandle<Self>){
        if state.failure.is_some(){return;}
        if let wl_seat::Event::Capabilities{capabilities}=event{
            match capabilities{
                WEnum::Value(caps)=>state.input.capabilities(seat,caps.contains(wl_seat::Capability::Keyboard),qh),
                _=>state.fail("unsupported seat capabilities"),
            }
        }
    }
}
impl Dispatch<wl_keyboard::WlKeyboard,()> for State{
    fn event(state:&mut Self,keyboard:&wl_keyboard::WlKeyboard,event:wl_keyboard::Event,_:&(),_:&Connection,_:&QueueHandle<Self>){
        if state.failure.is_some(){return;}
        // Only the exact configured root is eligible for focus. The
        // decoder still requires a future visible prompt readiness gate.
        let root=state.surface.as_ref().filter(|_|state.configured);
        if let Err(reason)=state.input.event(keyboard,event,root){state.fail(reason);}
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn revoke_clears_input_and_never_enables_unbound_entry(){
        let mut input=Input::default();let now=Instant::now();
        assert!(!input.entry.insert('x',now));
        input.revoke();assert!(!input.entry.has_input());assert!(!input.entry.insert('x',now));
    }
}
