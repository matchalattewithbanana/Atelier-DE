//! Optional notification inbox. Never starts or replaces a notification daemon.
use crate::notification_bus::{self,Request};
use gtk::{glib,prelude::*};
use serde::Deserialize;
use std::{cell::RefCell,collections::{BTreeMap,BTreeSet},rc::Rc,sync::mpsc,time::Duration};

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
    #[serde(default="normal_urgency")] urgency:u8,
    #[serde(default)] closed_reason:Option<u32>,
}
fn normal_urgency()->u8{1}
#[derive(Debug,Deserialize,PartialEq,Eq)]
struct Snapshot {
    schema:u32,instance:String,dnd:bool,
    #[serde(default)] manual_dnd:Option<bool>,
    #[serde(default)] pause_remaining_seconds:Option<u32>,
    active:Vec<Notice>,history:Vec<Notice>,
}
fn snapshot(json:&str)->Result<Snapshot,String>{
    let state:Snapshot=serde_json::from_str(json).map_err(|e|e.to_string())?;
    if state.schema!=1||state.instance.is_empty()||state.active.len()>128||state.history.len()>100||
        state.active.iter().chain(&state.history).any(|n|n.actions.len()>8||n.urgency>2||n.progress.is_some_and(|p|p>100)){
        return Err("Unsupported notification snapshot".into());
    }
    Ok(state)
}
#[derive(Default,Clone,Copy,PartialEq,Eq)]
enum Section{#[default] Active,History,All}
impl Section{
    fn includes(self,active:bool)->bool{matches!(self,Self::All)||(active&&self==Self::Active)||(!active&&self==Self::History)}
}
#[derive(Default,PartialEq,Eq)]
struct Filter { query:String,app:Option<String>,missed:bool,grouped:bool,section:Section }
impl Filter {
    fn matches(&self,n:&Notice)->bool{
        (!self.missed||n.missed)&&self.app.as_ref().is_none_or(|app|app==&n.app)&&
            (self.query.is_empty()||[&n.app,&n.summary,&n.body].iter().any(|s|s.to_lowercase().contains(&self.query)))
    }
}
fn groups<'a>(notices:&'a [Notice],filter:&Filter,active:bool)->Vec<(String,Vec<&'a Notice>)>{
    let mut selected:Vec<_>=notices.iter().filter(|n|filter.matches(n)).collect();
    // Active replacements may be newer than later insertions. Closed history
    // already follows close order, which is not necessarily revision order.
    if active{selected.sort_by_key(|n|std::cmp::Reverse((n.revision,n.id)));}
    if !filter.grouped{return vec![(String::new(),selected)];}
    let mut positions=BTreeMap::<String,usize>::new();
    let mut groups:Vec<(String,Vec<&Notice>)>=Vec::new();
    for n in selected{
        let i=*positions.entry(n.app.clone()).or_insert_with(||{groups.push((n.app.clone(),Vec::new()));groups.len()-1});
        groups[i].1.push(n);
    }
    groups
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
    card.add_css_class("atelier-inbox-card");
    if n.urgency==2{card.add_css_class("atelier-inbox-critical");}
    let header=gtk::Box::new(gtk::Orientation::Horizontal,8);
    let icon=gtk::Image::from_icon_name(safe_icon(&n.icon));icon.set_pixel_size(20);header.append(&icon);
    let app=text(if n.app.is_empty(){"Unknown application"}else{&n.app},"dim-label");app.set_hexpand(true);header.append(&app);
    if n.urgency==2{header.append(&text("Urgent","atelier-inbox-tag"));}
    if n.missed{header.append(&text("Missed","atelier-inbox-tag"));}
    if active{
        let close=gtk::Button::from_icon_name("window-close-symbolic");close.add_css_class("flat");close.set_tooltip_text(Some("Dismiss notification"));
        let (sender,status,instance,id,revision)=(sender.clone(),status.clone(),instance.to_owned(),n.id,n.revision);
        close.set_sensitive(revision!=0);
        close.connect_clicked(move |_|enqueue(&sender,Request::Dismiss(instance.clone(),id,revision),&status));
        header.append(&close);
    }
    card.append(&header);
    card.append(&text(&n.summary,"atelier-inbox-summary"));
    if !active{card.append(&text(match n.closed_reason{Some(1)=>"Expired",Some(2)=>"Dismissed",Some(3)=>"Closed by application",_=>"Closed"},"dim-label"));}
    if !n.body.is_empty(){
        let body=text(&n.body,"");body.set_selectable(true);card.append(&body);
    }
    if let Some(progress)=n.progress{
        let bar=gtk::ProgressBar::new();bar.set_fraction(f64::from(progress)/100.0);bar.set_show_text(true);bar.set_text(Some(&format!("{progress}%")));card.append(&bar);
    }
    if active&&n.revision!=0{
        let actions=gtk::Box::new(gtk::Orientation::Vertical,4);actions.add_css_class("atelier-inbox-actions");
        for action in &n.actions{
            let button=gtk::Button::with_label(&action.label);
            let (sender,status,instance,id,revision,key)=(sender.clone(),status.clone(),instance.to_owned(),n.id,n.revision,action.key.clone());
            button.connect_clicked(move |_|enqueue(&sender,Request::Invoke(instance.clone(),id,revision,key.clone()),&status));
            actions.append(&button);
        }
        if !n.actions.is_empty(){card.append(&actions);}
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
            if !filter.section.includes(active){continue;}
            let grouped=groups(notices,filter,active);
            let count:usize=grouped.iter().map(|(_,rows)|rows.len()).sum();
            self.root.append(&text(&format!("{} ({count})",if active{"Active"}else{"Closed history"}),"title-3"));
            if count==0{self.root.append(&text("No matching notifications","dim-label"));continue;}
            for (app,rows) in grouped{
                let container=gtk::Box::new(gtk::Orientation::Vertical,8);
                for n in &rows{container.append(&card(n,active,&snapshot.instance,sender,status));}
                if filter.grouped{
                    let title=format!("{} ({})",if app.is_empty(){"Unknown application"}else{&app},rows.len());
                    let expander=gtk::Expander::new(Some(&title));expander.add_css_class("atelier-inbox-group");expander.set_child(Some(&container));
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
fn read_filter(search:&gtk::Entry,picker:&gtk::ComboBoxText,missed:&gtk::CheckButton,grouped:&gtk::CheckButton,section:&gtk::ComboBoxText)->Filter{
    Filter{query:search.text().trim().to_lowercase(),app:picker.active_id().and_then(|id|id.strip_prefix("app:").map(str::to_owned)),missed:missed.is_active(),grouped:grouped.is_active(),section:match section.active_id().as_deref(){Some("history")=>Section::History,Some("all")=>Section::All,_=>Section::Active}}
}

pub fn show(app:&gtk::Application){
    if let Some(window)=app.windows().into_iter().find(|w|w.widget_name()=="atelier-inbox"){window.present();return;}
    let window=gtk::ApplicationWindow::builder().application(app).title("Atelier · Notifications").default_width(640).default_height(650).build();
    window.set_widget_name("atelier-inbox");
    let root=gtk::Box::new(gtk::Orientation::Vertical,10);
    root.add_css_class("atelier-inbox");
    root.set_margin_top(18);root.set_margin_bottom(18);root.set_margin_start(18);root.set_margin_end(18);
    root.append(&text("Notification centre","atelier-inbox-heading"));
    let overview=text("","dim-label");root.append(&overview);
    let status=text("Reading Atelier notification inbox…","dim-label");root.append(&status);
    let controls=gtk::Box::new(gtk::Orientation::Horizontal,8);
    let clear=gtk::Button::with_label("Clear history");clear.set_sensitive(false);
    let cancel_clear=gtk::Button::with_label("Cancel");cancel_clear.set_visible(false);
    let refresh=gtk::Button::with_label("Refresh");
    let dnd=gtk::Button::with_label("Do Not Disturb");dnd.set_sensitive(false);
    controls.append(&dnd);controls.append(&refresh);controls.append(&clear);controls.append(&cancel_clear);root.append(&controls);
    let pause_row=gtk::Box::new(gtk::Orientation::Horizontal,8);
    let pause_choice=gtk::ComboBoxText::new();
    for (id,label) in [("900","15 minutes"),("1800","30 minutes"),("3600","1 hour"),("7200","2 hours"),("28800","8 hours")]{pause_choice.append(Some(id),label);}
    pause_choice.set_active_id(Some("1800"));
    let pause=gtk::Button::with_label("Pause popups");pause.set_sensitive(false);
    let resume=gtk::Button::with_label("End pause");resume.set_sensitive(false);
    pause_row.append(&pause_choice);pause_row.append(&pause);pause_row.append(&resume);root.append(&pause_row);
    let pause_status=text("","dim-label");root.append(&pause_status);
    let search=gtk::Entry::new();search.set_placeholder_text(Some("Search applications, titles and messages"));root.append(&search);
    let picker=gtk::ComboBoxText::new();picker.append(Some("all"),"All applications");picker.set_active_id(Some("all"));root.append(&picker);
    let filters=gtk::Box::new(gtk::Orientation::Horizontal,8);
    let missed=gtk::CheckButton::with_label("Missed only");
    let grouped=gtk::CheckButton::with_label("Group by application");grouped.set_active(true);
    let section=gtk::ComboBoxText::new();
    for (id,label) in [("active","Active"),("history","History"),("all","All notifications")]{section.append(Some(id),label);}
    section.set_active_id(Some("active"));root.append(&section);
    filters.append(&missed);filters.append(&grouped);root.append(&filters);
    let mut inbox=Inbox::new();
    let scroll=gtk::ScrolledWindow::builder().vexpand(true).child(&inbox.root).build();root.append(&scroll);
    let (requests,replies,watch)=notification_bus::start("List",snapshot);
    let state=Rc::new(RefCell::new(None::<Snapshot>));
    type ReviewedHistory=(String,Vec<(u32,u64)>);
    let reviewed=Rc::new(RefCell::new(None::<ReviewedHistory>));
    {let (state,sender,status,reviewed,cancel)=(state.clone(),requests.clone(),status.clone(),reviewed.clone(),cancel_clear.downgrade());
        clear.connect_clicked(move |button|{
            let pending=reviewed.borrow_mut().take();
            if let Some((instance,entries))=pending{
                enqueue(&sender,Request::Clear(instance,entries),&status);
                button.set_label("Clear history");if let Some(cancel)=cancel.upgrade(){cancel.set_visible(false);}
                return;
            }
            if let Some(s)=state.borrow().as_ref(){
                let entries:Vec<_>=s.history.iter().map(|n|(n.id,n.revision)).collect();
                if entries.is_empty()||entries.iter().any(|(_,r)|*r==0){status.set_text("No revision-aware history to clear.");return;}
                button.set_label(&format!("Confirm clear ({})",entries.len()));
                *reviewed.borrow_mut()=Some((s.instance.clone(),entries));
                if let Some(cancel)=cancel.upgrade(){cancel.set_visible(true);}
                status.set_text("Confirm removal of all closed history, including filtered-out entries. Active notifications are unaffected.");
            }
        });
    }
    {let (reviewed,clear,status)=(reviewed.clone(),clear.downgrade(),status.downgrade());cancel_clear.connect_clicked(move |cancel|{
        reviewed.borrow_mut().take();cancel.set_visible(false);
        if let Some(clear)=clear.upgrade(){clear.set_label("Clear history");}
        if let Some(status)=status.upgrade(){status.set_text("History clear cancelled.");}
    });}
    {let (state,sender,status)=(state.clone(),requests.clone(),status.clone());dnd.connect_clicked(move |_|{
        if let Some(s)=state.borrow().as_ref(){enqueue(&sender,Request::Dnd(s.instance.clone(),!s.manual_dnd.unwrap_or(s.dnd)),&status);}
    });}
    for (button,cancel) in [(&pause,false),(&resume,true)]{
        let (state,sender,status,choice)=(state.clone(),requests.clone(),status.clone(),pause_choice.clone());
        button.connect_clicked(move |_|{
            let duration=if cancel{0}else{let Some(seconds)=choice.active_id().and_then(|id|id.parse::<u32>().ok())else{return;};seconds};
            if let Some(s)=state.borrow().as_ref(){enqueue(&sender,Request::Pause(s.instance.clone(),duration),&status);}
        });
    }
    {let sender=requests.clone();refresh.connect_clicked(move |_|{let _=sender.try_send(Request::Refresh);});}
    let weak=window.downgrade();
    let mut last_filter=Filter::default();
    glib::timeout_add_local(Duration::from_millis(100),move||{
        if weak.upgrade().is_none(){return glib::ControlFlow::Break;}
        let _=&watch;
        let filter=read_filter(&search,&picker,&missed,&grouped,&section);
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
                        clear.set_sensitive(!s.history.is_empty()&&s.history.iter().all(|n|n.revision!=0));
                        let stale=reviewed.borrow().as_ref().is_some_and(|(instance,entries)|instance!=&s.instance||entries.len()!=s.history.len()||!entries.iter().zip(&s.history).all(|((id,r),n)|*id==n.id&&*r==n.revision));
                        if stale{reviewed.borrow_mut().take();clear.set_label("Clear history");cancel_clear.set_visible(false);}
                    }
                    overview.set_text(&format!("{} active · {} in history · {} missed",s.active.len(),s.history.len(),s.active.iter().chain(&s.history).filter(|n|n.missed).count()));
                    if s.manual_dnd.unwrap_or(s.dnd){dnd.add_css_class("suggested-action");}else{dnd.remove_css_class("suggested-action");}
                    dnd.set_sensitive(true);dnd.set_label(if s.manual_dnd.unwrap_or(s.dnd){"Do Not Disturb: On"}else{"Do Not Disturb: Off"});
                    pause.set_sensitive(s.pause_remaining_seconds.is_some());
                    let remaining=s.pause_remaining_seconds.unwrap_or(0);
                    resume.set_sensitive(remaining>0);
                    pause_status.set_text(&if remaining>0{format!("Temporary pause: {}m {:02}s remaining",remaining/60,remaining%60)}else{"No temporary pause".into()});
                    *state.borrow_mut()=Some(s);
                    status.set_text(reply.action_error.as_deref().unwrap_or(if reviewed.borrow().is_some(){"Confirm removal of all closed history, including filtered-out entries. Active notifications are unaffected."}else{"Atelier inbox · content stays in session memory"}));
                }
                Err(error)=>{
                    state.borrow_mut().take();reviewed.borrow_mut().take();clear.set_label("Clear history");cancel_clear.set_visible(false);overview.set_text("");inbox.clear();clear.set_sensitive(false);dnd.set_sensitive(false);pause.set_sensitive(false);resume.set_sensitive(false);pause_status.set_text("");
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
        Notice{id,app:app.into(),summary:format!("Notice {id}"),body:"Backup completed".into(),revision:0,progress:None,missed,actions:vec![],icon:String::new(),urgency:1,closed_reason:None}
    }
    #[test]
    fn search_app_filter_grouping_and_missed_combine(){
        let notices=vec![notice(1,"B",false),notice(2,"A",true),notice(3,"A",false)];
        let filter=Filter{query:"backup".into(),app:Some("A".into()),missed:true,grouped:true,section:Section::All};
        let result=groups(&notices,&filter,true);
        assert_eq!(result.len(),1);assert_eq!(result[0].0,"A");assert_eq!(result[0].1[0].id,2);
        assert_eq!(groups(&notices,&Filter{grouped:true,..Filter::default()},true)[0].1.len(),2);
        assert!(groups(&notices,&Filter{query:"missing".into(),..Filter::default()},true)[0].1.is_empty());
    }
    #[test]
    fn active_groups_follow_latest_revision_and_history_keeps_close_order(){
        let mut old=notice(1,"A",false);old.revision=10;
        let mut newer=notice(2,"B",false);newer.revision=12;
        let mut replacement=notice(3,"A",false);replacement.revision=15;
        let notices=vec![old,newer,replacement];
        let filter=Filter{grouped:true,..Filter::default()};
        let active=groups(&notices,&filter,true);
        assert_eq!(active[0].0,"A");assert_eq!(active[0].1.iter().map(|n|n.id).collect::<Vec<_>>(),vec![3,1]);
        let history=groups(&notices,&filter,false);
        assert_eq!(history[0].1.iter().map(|n|n.id).collect::<Vec<_>>(),vec![1,3]);
        assert!(Section::Active.includes(true)&&!Section::Active.includes(false));
        assert!(Section::History.includes(false)&&!Section::History.includes(true));
        assert!(Section::All.includes(true)&&Section::All.includes(false));
    }
    #[test]
    fn icons_are_theme_names_not_paths(){
        assert_eq!(safe_icon("/tmp/icon"),"dialog-information-symbolic");
        assert_eq!(safe_icon("emblem-ok-symbolic"),"emblem-ok-symbolic");
    }
}
