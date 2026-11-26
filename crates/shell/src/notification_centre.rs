//! Optional notification inbox. Never starts or replaces a notification daemon.
use atelier_config::component;
use gtk::{glib,prelude::*};
use serde::Deserialize;
use std::{cell::RefCell,collections::{BTreeMap,BTreeSet},rc::Rc,process::Command,sync::mpsc,time::Duration};

#[derive(Clone,Debug,Deserialize,PartialEq,Eq)]
struct Action { key:String,label:String }
#[derive(Clone,Debug,Deserialize,PartialEq,Eq)]
struct Notice {
    id:u32,app:String,summary:String,body:String,
    #[serde(default)] revision:u64,
    #[serde(default)] progress:Option<u8>,
    #[serde(default)] missed:bool,
    #[serde(default)] actions:Vec<Action>,
    #[serde(default)] icon:String,
}
#[derive(Debug,Deserialize,PartialEq,Eq)]
struct Snapshot {
    schema:u32,instance:String,dnd:bool,
    #[serde(default)] manual_dnd:Option<bool>,
    #[serde(default)] pause_remaining_seconds:Option<u32>,
    active:Vec<Notice>,history:Vec<Notice>,
}
enum Request { Dismiss(String,u32),Invoke(String,u32,u64,String),Clear(String),Dnd(String,bool),Pause(String,String),Refresh }
struct Reply { snapshot:Result<Snapshot,String>,action_error:Option<String> }

pub(crate) fn call(args:&[&str])->Result<String,String>{
    let helper=component("atelier-notifications");
    let output=Command::new("timeout").args(["--kill-after=1s","3s"]).arg(helper.get_program()).args(args).output().map_err(|e|e.to_string())?;
    if !output.status.success(){return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());}
    String::from_utf8(output.stdout).map_err(|e|e.to_string())
}
fn snapshot()->Result<Snapshot,String>{
    let state:Snapshot=serde_json::from_str(&call(&["list"])?).map_err(|e|e.to_string())?;
    if state.schema!=1||state.instance.is_empty()||state.active.len()>128||state.history.len()>100||
        state.active.iter().chain(&state.history).any(|n|n.actions.len()>8||n.progress.is_some_and(|p|p>100)){
        return Err("Unsupported notification snapshot".into());
    }
    Ok(state)
}
#[derive(Default,PartialEq,Eq)]
struct Filter { query:String,app:Option<String>,missed:bool,grouped:bool }
impl Filter {
    fn matches(&self,n:&Notice)->bool{
        (!self.missed||n.missed)&&self.app.as_ref().is_none_or(|app|app==&n.app)&&
            (self.query.is_empty()||[&n.app,&n.summary,&n.body].iter().any(|s|s.to_lowercase().contains(&self.query)))
    }
}
fn groups<'a>(notices:&'a [Notice],filter:&Filter)->Vec<(String,Vec<&'a Notice>)>{
    let mut selected:Vec<_>=notices.iter().filter(|n|filter.matches(n)).collect();
    // Active snapshots are in insertion order, closed history is already newest first.
    // Preserve the supplied ordering within each group.
    if !filter.grouped{return vec![(String::new(),selected)];}
    let mut groups=BTreeMap::<String,Vec<&Notice>>::new();
    for n in selected.drain(..){groups.entry(n.app.clone()).or_default().push(n);}
    groups.into_iter().collect()
}
fn text(value:&str,class:&str)->gtk::Label{
    let label=gtk::Label::new(Some(value));label.set_xalign(0.0);label.set_wrap(true);label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_max_width_chars(60);
    if !class.is_empty(){label.add_css_class(class);}
    label
}
fn safe_icon(icon:&str)->&str{
    if !icon.is_empty()&&icon.len()<=128&&icon.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'-'|b'_')){icon}else{"dialog-information-symbolic"}
}
fn enqueue(sender:&mpsc::SyncSender<Request>,request:Request,status:&gtk::Label){
    if sender.try_send(request).is_err(){status.set_text("Notification service busy. Try again.");}
}
fn card(n:&Notice,active:bool,instance:&str,sender:&mpsc::SyncSender<Request>,status:&gtk::Label)->gtk::Box{
    let card=gtk::Box::new(gtk::Orientation::Vertical,6);
    card.add_css_class("atelier-notification-card");
    let header=gtk::Box::new(gtk::Orientation::Horizontal,8);
    let icon=gtk::Image::from_icon_name(safe_icon(&n.icon));icon.set_pixel_size(20);header.append(&icon);
    let app=text(if n.app.is_empty(){"Unknown application"}else{&n.app},"dim-label");app.set_hexpand(true);header.append(&app);
    if n.missed{header.append(&text("Missed","dim-label"));}
    if active{
        let close=gtk::Button::from_icon_name("window-close-symbolic");close.add_css_class("flat");close.set_tooltip_text(Some("Dismiss notification"));
        let (sender,status,instance,id)=(sender.clone(),status.clone(),instance.to_owned(),n.id);
        close.connect_clicked(move |_|enqueue(&sender,Request::Dismiss(instance.clone(),id),&status));
        header.append(&close);
    }
    card.append(&header);
    card.append(&text(&n.summary,"heading"));
    if !n.body.is_empty(){
        let body=text(&n.body,"");body.set_selectable(true);card.append(&body);
    }
    if let Some(progress)=n.progress{
        let bar=gtk::ProgressBar::new();bar.set_fraction(f64::from(progress)/100.0);bar.set_show_text(true);bar.set_text(Some(&format!("{progress}%")));card.append(&bar);
    }
    if active&&n.revision!=0{
        for action in &n.actions{
            let button=gtk::Button::with_label(&action.label);
            let (sender,status,instance,id,revision,key)=(sender.clone(),status.clone(),instance.to_owned(),n.id,n.revision,action.key.clone());
            button.connect_clicked(move |_|enqueue(&sender,Request::Invoke(instance.clone(),id,revision,key.clone()),&status));
            card.append(&button);
        }
    }
    card
}
/// A section is rebuilt only on a content/filter change. Remember group
/// expansion by section and exact app name for the same daemon instance.
struct Inbox {
    root:gtk::Box,
    expanded:BTreeMap<(bool,String),gtk::Expander>,
}
impl Inbox{
    fn new()->Self{
        let root=gtk::Box::new(gtk::Orientation::Vertical,12);
        Self{root,expanded:BTreeMap::new()}
    }
    fn clear(&mut self){
        self.expanded.clear();
        while let Some(child)=self.root.first_child(){self.root.remove(&child);}
    }
    fn render(&mut self,snapshot:&Snapshot,filter:&Filter,sender:&mpsc::SyncSender<Request>,status:&gtk::Label,same_instance:bool){
        let previous:BTreeMap<_,_>=if same_instance{self.expanded.iter().map(|(key,value)|(key.clone(),value.is_expanded())).collect()}else{BTreeMap::new()};
        self.clear();
        for (active,notices) in [(true,&snapshot.active),(false,&snapshot.history)]{
            let grouped=groups(notices,filter);
            let count:usize=grouped.iter().map(|(_,rows)|rows.len()).sum();
            self.root.append(&text(&format!("{} ({count})",if active{"Active"}else{"Closed history"}),"title-3"));
            if count==0{self.root.append(&text("No matching notifications","dim-label"));continue;}
            for (app,rows) in grouped{
                let container=gtk::Box::new(gtk::Orientation::Vertical,8);
                if active{
                    for n in rows.iter().rev(){container.append(&card(n,true,&snapshot.instance,sender,status));}
                }else{
                    for n in &rows{container.append(&card(n,false,&snapshot.instance,sender,status));}
                }
                if filter.grouped{
                    let title=format!("{} ({})",if app.is_empty(){"Unknown application"}else{&app},rows.len());
                    let expander=gtk::Expander::new(Some(&title));expander.set_child(Some(&container));
                    let key=(active,app);
                    expander.set_expanded(previous.get(&key).copied().unwrap_or(active));
                    self.root.append(&expander);self.expanded.insert(key,expander);
                }else{self.root.append(&container);}
            }
        }
    }
}
fn refill_apps(picker:&gtk::ComboBoxText,snapshot:&Snapshot){
    let old=picker.active_id();
    let apps:BTreeSet<_>=snapshot.active.iter().chain(&snapshot.history).map(|n|n.app.as_str()).collect();
    picker.remove_all();picker.append(Some("all"),"All applications");
    for app in apps{picker.append(Some(&format!("app:{app}")),if app.is_empty(){"Unknown application"}else{app});}
    if !old.as_ref().is_some_and(|id|picker.set_active_id(Some(id.as_str()))){picker.set_active_id(Some("all"));}
}
fn read_filter(search:&gtk::SearchEntry,picker:&gtk::ComboBoxText,missed:&gtk::CheckButton,grouped:&gtk::CheckButton)->Filter{
    Filter{query:search.text().trim().to_lowercase(),app:picker.active_id().and_then(|id|id.strip_prefix("app:").map(str::to_owned)),missed:missed.is_active(),grouped:grouped.is_active()}
}

pub fn show(app:&gtk::Application){
    if let Some(window)=app.windows().into_iter().find(|w|w.widget_name()=="atelier-inbox"){window.present();return;}
    let window=gtk::ApplicationWindow::builder().application(app).title("Atelier · Notifications").default_width(640).default_height(650).build();
    window.set_widget_name("atelier-inbox");
    let root=gtk::Box::new(gtk::Orientation::Vertical,10);
    root.set_margin_top(18);root.set_margin_bottom(18);root.set_margin_start(18);root.set_margin_end(18);
    root.append(&text("Notification centre","title-1"));
    let status=text("Reading Atelier notification inbox…","dim-label");root.append(&status);
    let controls=gtk::Box::new(gtk::Orientation::Horizontal,8);
    let clear=gtk::Button::with_label("Clear history");clear.set_sensitive(false);
    let refresh=gtk::Button::with_label("Refresh");
    let dnd=gtk::Button::with_label("Do Not Disturb");dnd.set_sensitive(false);
    controls.append(&dnd);controls.append(&refresh);controls.append(&clear);root.append(&controls);
    let pause_row=gtk::Box::new(gtk::Orientation::Horizontal,8);
    let pause_choice=gtk::ComboBoxText::new();
    for (id,label) in [("15m","15 minutes"),("30m","30 minutes"),("1h","1 hour"),("2h","2 hours"),("8h","8 hours")]{pause_choice.append(Some(id),label);}
    pause_choice.set_active_id(Some("30m"));
    let pause=gtk::Button::with_label("Pause popups");pause.set_sensitive(false);
    let resume=gtk::Button::with_label("End pause");resume.set_sensitive(false);
    pause_row.append(&pause_choice);pause_row.append(&pause);pause_row.append(&resume);root.append(&pause_row);
    let pause_status=text("","dim-label");root.append(&pause_status);
    let search=gtk::SearchEntry::new();search.set_placeholder_text(Some("Search applications, titles and messages"));root.append(&search);
    let picker=gtk::ComboBoxText::new();picker.append(Some("all"),"All applications");picker.set_active_id(Some("all"));root.append(&picker);
    let filters=gtk::Box::new(gtk::Orientation::Horizontal,8);
    let missed=gtk::CheckButton::with_label("Missed only");
    let grouped=gtk::CheckButton::with_label("Group by application");
    filters.append(&missed);filters.append(&grouped);root.append(&filters);
    let mut inbox=Inbox::new();
    let scroll=gtk::ScrolledWindow::builder().vexpand(true).child(&inbox.root).build();root.append(&scroll);
    let (requests,incoming)=mpsc::sync_channel::<Request>(1);
    let (outgoing,replies)=mpsc::sync_channel::<Reply>(2);
    std::thread::spawn(move||{
        let mut action_error=None;
        loop{
            if outgoing.send(Reply{snapshot:snapshot(),action_error:action_error.take()}).is_err(){break;}
            let result=match incoming.recv_timeout(Duration::from_secs(3)){
                Ok(Request::Dismiss(instance,id))=>call(&["dismiss",&instance,&id.to_string()]),
                Ok(Request::Invoke(instance,id,revision,key))=>call(&["invoke",&instance,&id.to_string(),&revision.to_string(),&key]),
                Ok(Request::Clear(instance))=>call(&["clear-history",&instance]),
                Ok(Request::Dnd(instance,enabled))=>call(&["dnd",&instance,if enabled{"on"}else{"off"}]),
                Ok(Request::Pause(instance,duration))=>call(&["pause",&instance,&duration]),
                Ok(Request::Refresh)|Err(mpsc::RecvTimeoutError::Timeout)=>continue,
                Err(mpsc::RecvTimeoutError::Disconnected)=>break,
            };
            action_error=result.err();
        }
    });
    let state=Rc::new(RefCell::new(None::<Snapshot>));
    {let (state,sender,status)=(state.clone(),requests.clone(),status.clone());clear.connect_clicked(move |_|{
        if let Some(s)=state.borrow().as_ref(){enqueue(&sender,Request::Clear(s.instance.clone()),&status);}
    });}
    {let (state,sender,status)=(state.clone(),requests.clone(),status.clone());dnd.connect_clicked(move |_|{
        if let Some(s)=state.borrow().as_ref(){enqueue(&sender,Request::Dnd(s.instance.clone(),!s.manual_dnd.unwrap_or(s.dnd)),&status);}
    });}
    for (button,cancel) in [(&pause,false),(&resume,true)]{
        let (state,sender,status,choice)=(state.clone(),requests.clone(),status.clone(),pause_choice.clone());
        button.connect_clicked(move |_|{
            let duration=if cancel{"off".into()}else{let Some(id)=choice.active_id()else{return;};id.to_string()};
            if let Some(s)=state.borrow().as_ref(){enqueue(&sender,Request::Pause(s.instance.clone(),duration),&status);}
        });
    }
    {let sender=requests.clone();refresh.connect_clicked(move |_|{let _=sender.try_send(Request::Refresh);});}
    let weak=window.downgrade();
    let mut last_filter=Filter::default();
    glib::timeout_add_local(Duration::from_millis(100),move||{
        if weak.upgrade().is_none(){return glib::ControlFlow::Break;}
        let filter=read_filter(&search,&picker,&missed,&grouped);
        if filter!=last_filter{
            if let Some(s)=state.borrow().as_ref(){inbox.render(s,&filter,&requests,&status,true);}
            last_filter=filter;
        }
        while let Ok(reply)=replies.try_recv(){
            match reply.snapshot{
                Ok(s)=>{
                    let same_instance=state.borrow().as_ref().is_some_and(|old|old.instance==s.instance);
                    let changed=state.borrow().as_ref().is_none_or(|old|!same_instance||old.active!=s.active||old.history!=s.history);
                    if changed{
                        refill_apps(&picker,&s);
                        last_filter=read_filter(&search,&picker,&missed,&grouped);
                        inbox.render(&s,&last_filter,&requests,&status,same_instance);
                        clear.set_sensitive(!s.history.is_empty());
                    }
                    dnd.set_sensitive(true);dnd.set_label(if s.manual_dnd.unwrap_or(s.dnd){"Do Not Disturb: On"}else{"Do Not Disturb: Off"});
                    pause.set_sensitive(s.pause_remaining_seconds.is_some());
                    let remaining=s.pause_remaining_seconds.unwrap_or(0);
                    resume.set_sensitive(remaining>0);
                    pause_status.set_text(&if remaining>0{format!("Temporary pause: {}m {:02}s remaining",remaining/60,remaining%60)}else{"No temporary pause".into()});
                    *state.borrow_mut()=Some(s);
                    status.set_text(reply.action_error.as_deref().unwrap_or("Atelier inbox · content stays in session memory"));
                }
                Err(error)=>{
                    state.borrow_mut().take();inbox.clear();clear.set_sensitive(false);dnd.set_sensitive(false);pause.set_sensitive(false);resume.set_sensitive(false);pause_status.set_text("");
                    picker.remove_all();picker.append(Some("all"),"All applications");picker.set_active_id(Some("all"));
                    inbox.root.append(&text("No Atelier inbox available. Your existing notification service is untouched.","dim-label"));
                    status.set_text(&format!("Atelier notification service unavailable: {error}"));
                }
            }
        }
        glib::ControlFlow::Continue
    });
    window.set_child(Some(&root));window.present();
}

#[cfg(test)]
mod tests{
    use super::*;
    fn notice(id:u32,app:&str,missed:bool)->Notice{
        Notice{id,app:app.into(),summary:format!("Notice {id}"),body:"Backup completed".into(),revision:0,progress:None,missed,actions:vec![],icon:String::new()}
    }
    #[test]
    fn search_app_filter_grouping_and_missed_combine(){
        let notices=vec![notice(1,"B",false),notice(2,"A",true),notice(3,"A",false)];
        let filter=Filter{query:"backup".into(),app:Some("A".into()),missed:true,grouped:true};
        let result=groups(&notices,&filter);
        assert_eq!(result.len(),1);assert_eq!(result[0].0,"A");assert_eq!(result[0].1[0].id,2);
        assert_eq!(groups(&notices,&Filter{grouped:true,..Filter::default()})[0].1.len(),2);
        assert!(groups(&notices,&Filter{query:"missing".into(),..Filter::default()})[0].1.is_empty());
    }
    #[test]
    fn icons_are_theme_names_not_paths(){
        assert_eq!(safe_icon("/tmp/icon"),"dialog-information-symbolic");
        assert_eq!(safe_icon("emblem-ok-symbolic"),"emblem-ok-symbolic");
    }
}
