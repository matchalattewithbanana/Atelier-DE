//! Mapped-card-only PulseAudio invalidations. Never infer state from events.
use gtk::{gio,glib,prelude::*};
use std::{cell::{Cell,RefCell},rc::Rc,time::Duration,ffi::OsStr};

fn relevant(line:&str)->bool{
    // pactl's C-locale subscription format. Ignore high-frequency stream,
    // client and module changes; card/server/device changes invalidate inventory.
    let Some((event,object))=line.strip_prefix("Event '").and_then(|s|s.split_once("' on "))else{return false;};
    if !matches!(event,"new"|"change"|"remove"){return false;}
    let kind=object.split_whitespace().next().unwrap_or("");
    matches!(kind,"sink"|"source"|"server"|"card")
}
struct Subscription{
    process:gio::Subprocess,
    task:glib::JoinHandle<()>,
    alive:Rc<Cell<bool>>,
}
impl Drop for Subscription{
    fn drop(&mut self){self.task.abort();if self.alive.get(){self.process.force_exit();}}
}
fn subscribe(invalidate:Rc<dyn Fn()>)->Option<Subscription>{
    let launcher=gio::SubprocessLauncher::new(gio::SubprocessFlags::STDOUT_PIPE|gio::SubprocessFlags::STDERR_SILENCE);
    launcher.setenv("LC_ALL","C",true);
    let process=launcher.spawn(&[OsStr::new("pactl"),OsStr::new("subscribe")]).ok()?;
    let Some(stdout)=process.stdout_pipe()else{process.force_exit();return None;};
    let input=gio::DataInputStream::new(&stdout);
    let alive=Rc::new(Cell::new(true));
    let active=alive.clone();let child=process.clone();
    let task=glib::MainContext::default().spawn_local(async move{
        while let Ok(Some(line))=input.read_line_utf8_future(glib::Priority::DEFAULT).await{
            if relevant(&line){invalidate();}
        }
        // A read error is not necessarily EOF. Stop/reap this child before
        // the next compatibility retry creates another subscription.
        child.force_exit();let _=child.wait_future().await;
        active.set(false);invalidate();
    });
    Some(Subscription{process,task,alive})
}
#[derive(Default)]
struct Lifecycle{epoch:u64,subscription:Option<Subscription>}
/// The only owner is the mapped widget. Dropping or unmapping it stops the
/// subscription, including when a popover is cached for later use.
pub fn attach(card:&gtk::Box,refresh:Rc<dyn Fn()>){
    let life=Rc::new(RefCell::new(Lifecycle::default()));
    let state=life.clone();
    card.connect_map(move|card|{
        let epoch={
            let mut state=state.borrow_mut();
            state.subscription.take();state.epoch=state.epoch.wrapping_add(1);state.epoch
        };
        let weak=card.downgrade();let life=Rc::downgrade(&state);
        let queued=Rc::new(Cell::new(false));
        let refresh=refresh.clone();
        let invalidate:Rc<dyn Fn()>=Rc::new(move||{
            if queued.replace(true){return;}
            let (weak,life,queued,refresh)=(weak.clone(),life.clone(),queued.clone(),refresh.clone());
            glib::timeout_add_local_once(Duration::from_millis(350),move||{
                queued.set(false);
                if weak.upgrade().is_some_and(|c|c.is_mapped())&&life.upgrade().is_some_and(|s|s.borrow().epoch==epoch){refresh();}
            });
        });
        state.borrow_mut().subscription=subscribe(invalidate.clone());
        let weak=card.downgrade();let life=Rc::downgrade(&state);
        glib::timeout_add_local(Duration::from_secs(15),move||{
            let (Some(card),Some(life))=(weak.upgrade(),life.upgrade())else{return glib::ControlFlow::Break;};
            if !card.is_mapped()||life.borrow().epoch!=epoch{return glib::ControlFlow::Break;}
            let mut state=life.borrow_mut();
            if state.subscription.as_ref().is_none_or(|s|!s.alive.get()){
                state.subscription.take();state.subscription=subscribe(invalidate.clone());
            }
            drop(state);invalidate();glib::ControlFlow::Continue
        });
    });
    card.connect_unmap(move |_|{
        let mut life=life.borrow_mut();life.epoch=life.epoch.wrapping_add(1);life.subscription.take();
    });
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn filters_relevant_inventory_changes_not_stream_activity(){
        for line in ["Event 'change' on sink #2","Event 'new' on source #1","Event 'remove' on card #4","Event 'change' on server #4294967295"]{assert!(relevant(line));}
        for line in ["Event 'change' on sink-input #2","Event 'change' on source-output #2","Event 'new' on client #1","Event 'bad' on sink #1","sink #2",""]{assert!(!relevant(line));}
    }
}
