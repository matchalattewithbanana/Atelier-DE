//! Session lock ownership and presentation barrier. The protocol adapter
//! must authorize the exact resource before forwarding any state changes.
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub(crate) struct ProtectedFrame(u64);

#[derive(Debug,PartialEq,Eq)]
enum Phase<Owner>{
    Desktop,
    Pending{owner:Owner,epoch:u64},
    Locked{owner:Owner},
    Orphaned,
}

/// The exact protocol object, not its client, application ID or a serial,
/// must own both lock acceptance and unlock. Losing the owner fails closed.
pub(crate) struct Lifecycle<Owner>{
    phase:Phase<Owner>,
    epoch:u64,
}
impl<Owner> Default for Lifecycle<Owner>{
    fn default()->Self{Self{phase:Phase::Desktop,epoch:0}}
}
impl<Owner:Eq> Lifecycle<Owner>{
    pub(crate) fn desktop_visible(&self)->bool{matches!(self.phase,Phase::Desktop)}

    // Only the eventual guarded protocol adapter will call these transition
    // methods. Keeping the global disabled makes the unfinished path unreachable.
    #[allow(dead_code)]
    pub(crate) fn begin(&mut self,owner:Owner)->bool{
        if !self.desktop_visible(){return false;}
        let Some(epoch)=self.epoch.checked_add(1)else{return false;};
        self.epoch=epoch;
        self.phase=Phase::Pending{owner,epoch};true
    }
    pub(crate) fn protected_frame(&self)->Option<ProtectedFrame>{
        match &self.phase{
            Phase::Pending{epoch,..}=>Some(ProtectedFrame(*epoch)),
            _=>None,
        }
    }
    /// Accept only a token captured when the protected frame was submitted
    /// and observed at actual page-flip completion, never an earlier desktop flip.
    pub(crate) fn presented(&mut self,frame:ProtectedFrame)->bool{
        if self.protected_frame()!=Some(frame){return false;}
        let old=std::mem::replace(&mut self.phase,Phase::Orphaned);
        if let Phase::Pending{owner,..}=old{
            self.phase=Phase::Locked{owner};true
        }else{false}
    }
    #[allow(dead_code)]
    pub(crate) fn owns_request(&self,owner:&Owner)->bool{
        matches!(&self.phase,Phase::Pending{owner:current,..}|Phase::Locked{owner:current} if current==owner)
    }
    pub(crate) fn can_unlock(&self,owner:&Owner)->bool{
        matches!(&self.phase,Phase::Locked{owner:current} if current==owner)
    }
    pub(crate) fn unlock(&mut self,owner:&Owner)->bool{
        if !self.can_unlock(owner){return false;}
        self.phase=Phase::Desktop;true
    }
    #[allow(dead_code)]
    pub(crate) fn disconnected(&mut self,owner:&Owner)->bool{
        if !self.owns_request(owner){return false;}
        self.phase=Phase::Orphaned;true
    }
}

/// Aggregate physical devices into one logical Wayland seat without
/// releasing a key/button while another device still holds it.
struct Controls<K>{
    holders:std::collections::HashMap<K,std::collections::HashSet<String>>,
}
impl<K> Default for Controls<K>{
    fn default()->Self{Self{holders:Default::default()}}
}
impl<K:Eq+std::hash::Hash+Copy> Controls<K>{
    /// True only for the first press or final release of this control.
    fn change(&mut self,device:&str,code:K,pressed:bool)->bool{
        if pressed{
            let holders=self.holders.entry(code).or_default();
            let first=holders.is_empty();
            holders.insert(device.to_owned());
            first
        }else{
            let Some(holders)=self.holders.get_mut(&code)else{return false;};
            if !holders.remove(device)||!holders.is_empty(){return false;}
            self.holders.remove(&code);true
        }
    }
    fn remove(&mut self,device:&str)->Vec<K>{
        let mut released=Vec::new();
        self.holders.retain(|code,holders|{
            if holders.remove(device)&&holders.is_empty(){released.push(*code);false}else{true}
        });
        released
    }
    fn is_empty(&self)->bool{self.holders.is_empty()}
}

/// Physical controls are tracked independently of client focus. A boundary
/// quarantines input until every device's held controls are released. A
/// caller samples waiting() before updating, so the final release is still
/// dispatched without focus before restoration.
pub(crate) struct InputGate<K>{
    keys:Controls<K>,
    buttons:Controls<u32>,
    waiting:bool,
}
impl<K> Default for InputGate<K>{
    fn default()->Self{Self{keys:Default::default(),buttons:Default::default(),waiting:false}}
}
impl<K:Eq+std::hash::Hash+Copy> InputGate<K>{
    pub(crate) fn waiting(&self)->bool{self.waiting}
    pub(crate) fn boundary(&mut self){self.waiting=!self.keys.is_empty()||!self.buttons.is_empty();}
    pub(crate) fn key(&mut self,device:&str,key:K,pressed:bool)->bool{
        let changed=self.keys.change(device,key,pressed);
        self.recheck();changed
    }
    pub(crate) fn button(&mut self,device:&str,button:u32,pressed:bool)->bool{
        let changed=self.buttons.change(device,button,pressed);
        self.recheck();changed
    }
    /// A removed device no longer holds any controls. Return only releases
    /// that the seat must observe; other devices retain shared controls.
    pub(crate) fn remove_device(&mut self,device:&str)->(Vec<K>,Vec<u32>){
        let keys=self.keys.remove(device);let buttons=self.buttons.remove(device);
        self.recheck();(keys,buttons)
    }
    fn recheck(&mut self){
        if self.keys.is_empty()&&self.buttons.is_empty(){self.waiting=false;}
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn lock_ack_requires_completed_protected_frame_and_exact_owner(){
        let mut state=Lifecycle::default();
        assert!(state.desktop_visible());
        assert!(state.begin(1));
        assert!(!state.desktop_visible());
        assert!(!state.begin(2));
        assert!(state.owns_request(&1));assert!(!state.owns_request(&2));
        assert!(!state.can_unlock(&1));assert!(!state.unlock(&1));
        let frame=state.protected_frame().unwrap();
        assert!(!state.presented(ProtectedFrame(0)));
        assert!(state.presented(frame));
        assert!(!state.presented(frame));
        assert!(!state.can_unlock(&2));assert!(state.can_unlock(&1));assert!(!state.unlock(&2));
        assert!(!state.desktop_visible());
        assert!(state.unlock(&1));assert!(state.desktop_visible());
    }
    #[test]
    fn stale_flip_cannot_confirm_a_later_request(){
        let mut state=Lifecycle::default();
        assert!(state.begin(1));let old=state.protected_frame().unwrap();
        assert!(state.presented(old));assert!(state.unlock(&1));
        assert!(state.begin(2));let current=state.protected_frame().unwrap();
        assert_ne!(old,current);assert!(!state.presented(old));
        assert!(!state.unlock(&2));assert!(state.presented(current));assert!(state.unlock(&2));
    }
    #[test]
    fn owner_loss_is_fail_closed_even_before_ack(){
        for confirmed in [false,true]{
            let mut state=Lifecycle::default();assert!(state.begin(1));
            let frame=state.protected_frame().unwrap();
            if confirmed{assert!(state.presented(frame));}
            assert!(!state.disconnected(&2));assert!(state.disconnected(&1));
            assert!(!state.desktop_visible());assert!(!state.begin(2));
            assert!(!state.unlock(&1));assert!(!state.presented(frame));
            assert!(state.protected_frame().is_none());
        }
    }
    #[test]
    fn input_boundary_waits_for_every_control_including_new_presses(){
        let mut gate=InputGate::default();
        gate.key("a",1,true);gate.button("b",272,true);
        gate.boundary();assert!(gate.waiting());
        gate.key("a",1,false);assert!(gate.waiting());
        gate.key("a",2,true);gate.button("b",272,false);assert!(gate.waiting());
        gate.key("a",2,false);assert!(!gate.waiting());
        gate.boundary();assert!(!gate.waiting());
    }
    #[test]
    fn aggregate_controls_across_devices_and_ignore_duplicates(){
        let mut gate=InputGate::default();
        assert!(gate.key("a",1,true));assert!(!gate.key("a",1,true));
        assert!(!gate.key("b",1,true));
        assert!(gate.button("a",272,true));assert!(!gate.button("b",272,true));
        gate.boundary();assert!(gate.waiting());
        assert!(!gate.key("a",1,false));assert!(!gate.button("a",272,false));
        assert!(gate.waiting());
        assert!(!gate.key("unknown",1,false));assert!(!gate.button("unknown",272,false));
        assert!(gate.key("b",1,false));assert!(gate.waiting());
        assert!(gate.button("b",272,false));assert!(!gate.waiting());
    }
    #[test]
    fn removing_one_device_releases_only_its_last_held_controls(){
        let mut gate=InputGate::default();
        gate.key("a",1,true);gate.key("b",1,true);
        gate.key("a",2,true);gate.button("a",272,true);gate.button("b",272,true);
        gate.boundary();
        assert_eq!(gate.remove_device("a"),(vec![2],vec![]));assert!(gate.waiting());
        assert_eq!(gate.remove_device("a"),(vec![],vec![]));assert!(gate.waiting());
        assert_eq!(gate.remove_device("b"),(vec![1],vec![272]));assert!(!gate.waiting());
        assert!(gate.key("a",1,true)); // backend IDs may be reused after removal
    }
    #[test]
    fn generation_exhaustion_cannot_wrap(){
        let mut state=Lifecycle{phase:Phase::Desktop,epoch:u64::MAX};
        assert!(!state.begin(1));assert!(state.desktop_visible());
    }
}
