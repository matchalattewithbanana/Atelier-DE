//! Opening fades use monotonic elapsed time, not frame counts. Track only
//! live visible windows and never retain buffers after a client unmaps.
use std::time::{Duration,Instant};
const MAX_ACTIVE:usize=64;
const INITIAL_ALPHA:f32=0.55;
struct Fade<K>{
    key:K,start:Instant,duration:Duration,factor:f32,
}
pub struct OpeningFades<K>{
    active:Vec<Fade<K>>,
}
impl<K> Default for OpeningFades<K>{
    fn default()->Self{Self{active:Vec::new()}}
}
impl<K:PartialEq> OpeningFades<K>{
    pub fn start(&mut self,key:K,now:Instant,duration:Duration){
        self.cancel(&key);
        if duration.is_zero()||self.active.len()>=MAX_ACTIVE{return;}
        self.active.push(Fade{key,start:now,duration,factor:INITIAL_ALPHA});
    }
    pub fn factor(&self,key:&K)->f32{
        self.active.iter().find(|fade|&fade.key==key).map_or(1.0,|fade|fade.factor)
    }
    /// True while a visual change needs presenting, including the final
    /// fully opaque frame. False on all subsequent idle ticks.
    pub fn tick(&mut self,now:Instant)->bool{
        let mut changed=false;
        self.active.retain_mut(|fade|{
            let elapsed=now.saturating_duration_since(fade.start);
            if elapsed>=fade.duration{changed=true;return false;}
            let progress=(elapsed.as_secs_f32()/fade.duration.as_secs_f32()).clamp(0.0,1.0);
            let next=INITIAL_ALPHA+(1.0-INITIAL_ALPHA)*(1.0-(1.0-progress).powi(3));
            changed|=next!=fade.factor;
            fade.factor=next;
            true
        });
        changed
    }
    pub fn cancel(&mut self,key:&K){self.active.retain(|fade|&fade.key!=key);}
    pub fn clear(&mut self){self.active.clear();}
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn monotonic_fade_requests_final_frame_then_stops(){
        let mut fades=OpeningFades::default();let now=Instant::now();let duration=Duration::from_millis(200);
        fades.start(1,now,duration);
        assert_eq!(fades.factor(&1),INITIAL_ALPHA);assert!(!fades.tick(now));
        assert!(fades.tick(now+Duration::from_millis(50)));
        let first=fades.factor(&1);assert!(first>INITIAL_ALPHA&&first<1.0);
        assert!(fades.tick(now+Duration::from_millis(100)));assert!(fades.factor(&1)>first);
        assert!(fades.tick(now+duration));assert_eq!(fades.factor(&1),1.0);
        assert!(!fades.tick(now+duration+Duration::from_secs(1)));
    }
    #[test]
    fn disabled_cancelled_and_capped_fades_do_not_linger(){
        let mut fades=OpeningFades::default();let now=Instant::now();
        fades.start(0,now,Duration::ZERO);assert_eq!(fades.factor(&0),1.0);
        for i in 0..MAX_ACTIVE+1{fades.start(i,now,Duration::from_millis(200));}
        assert_eq!(fades.factor(&MAX_ACTIVE),1.0);
        fades.cancel(&0);assert_eq!(fades.factor(&0),1.0);
        fades.clear();assert!(!fades.tick(now+Duration::from_secs(1)));
    }
}
