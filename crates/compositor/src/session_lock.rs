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
    fn generation_exhaustion_cannot_wrap(){
        let mut state=Lifecycle{phase:Phase::Desktop,epoch:u64::MAX};
        assert!(!state.begin(1));assert!(state.desktop_visible());
    }
}
