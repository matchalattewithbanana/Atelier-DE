//! Workspace presentation is independent of compositor command transport.
//! Confirmed active tags are separate from decorative spring geometry.
use crate::workspace_backend::{Snapshot,WorkspaceBackend};
use atelier_config::design::{MotionKind,Scheme,Speed,SpringValue};
use gtk::{glib,prelude::*};
use std::{cell::{Cell,RefCell},rc::Rc};

struct Marks{
    springs:Vec<SpringValue>,
    confirmed:Option<[bool;9]>,
}
impl Marks{
    fn new(count:usize)->Self{
        let spring=Scheme::Expressive.spring(Speed::Fast,MotionKind::Spatial);
        Self{springs:vec![SpringValue::new(spring,0.0);count.min(9)],confirmed:None}
    }
    fn update(&mut self,state:Option<&Snapshot>,reduced:bool){
        let first=self.confirmed.is_none();
        self.confirmed=state.map(|s|s.active);
        for (i,spring) in self.springs.iter_mut().enumerate(){
            let active=self.confirmed.is_some_and(|a|a[i]);
            spring.retarget(if active{1.0}else{0.0});
            // Do not retain stale highlights on provider failure, or animate
            // an invented previous state on initial discovery/reconnection.
            if first||state.is_none()||reduced{spring.advance(0.0,true);}
        }
    }
    fn advance(&mut self,seconds:f64,reduced:bool)->bool{
        for s in &mut self.springs{s.advance(seconds,reduced);}
        self.springs.iter().any(|s|!s.settled())
    }
    fn moving(&self)->bool{self.springs.iter().any(|s|!s.settled())}
    fn width(&self,index:usize,cell:f64)->f64{
        if cell<=0.0||!cell.is_finite(){return 0.0;}
        let Some(s)=self.springs.get(index)else{return 0.0;};
        if self.confirmed.is_none(){return 0.0;}
        // Clamp overshoot inside its own cell; no animated geometry enters
        // another workspace's hit target.
        let max=(cell-6.0).clamp(0.0,26.0);
        max*s.value().clamp(0.0,1.0)
    }
}
fn reduced(local:bool)->bool{
    local||gtk::Settings::default().is_some_and(|s|!s.property::<bool>("gtk-enable-animations"))
}
pub struct Strip{
    pub root:gtk::Box,
    buttons:Vec<gtk::Button>,
    marks:Rc<RefCell<Marks>>,
    area:gtk::DrawingArea,
    running:Rc<Cell<bool>>,
    local_reduced:Rc<Cell<bool>>,
}
impl Strip{
    pub fn new(backend:Rc<WorkspaceBackend>)->Self{
        let root=gtk::Box::new(gtk::Orientation::Vertical,0);
        root.add_css_class("atelier-workspaces");root.set_valign(gtk::Align::Center);
        let row=gtk::Box::new(gtk::Orientation::Horizontal,0);row.set_homogeneous(true);row.set_direction(gtk::TextDirection::Ltr);
        let mut buttons=Vec::new();
        for index in 1..=backend.count() as u8{
            let button=gtk::Button::with_label(&index.to_string());
            button.add_css_class("flat");button.add_css_class("atelier-workspace-button");
            button.set_sensitive(false);
            let description=format!("Workspace {index} · Unavailable");
            button.set_tooltip_text(Some(&description));
            button.update_property(&[gtk::accessible::Property::Label(&description)]);
            let target=backend.clone();button.connect_clicked(move |_|target.focus(index));
            row.append(&button);buttons.push(button);
        }
        let marks=Rc::new(RefCell::new(Marks::new(buttons.len())));
        let area=gtk::DrawingArea::new();area.set_content_height(7);area.set_hexpand(true);area.set_can_target(false);
        area.add_css_class("atelier-workspace-mark");
        let draw=marks.clone();
        area.set_draw_func(move|widget,cr,w,h|{
            let marks=draw.borrow();let count=marks.springs.len();
            if count==0||h<2||w<1{return;}
            let cell=f64::from(w)/count as f64;
            let color=widget.style_context().color();
            cr.set_source_rgba(f64::from(color.red()),f64::from(color.green()),f64::from(color.blue()),f64::from(color.alpha()));
            cr.set_line_width(4.0f64.min(f64::from(h)));cr.set_line_cap(gtk::cairo::LineCap::Round);
            for i in 0..count{
                let width=marks.width(i,cell);
                if width<=0.5{continue;}
                // Line caps remain within the cell as well.
                let half=(width-4.0).max(0.0)/2.0;
                let center=(i as f64+0.5)*cell;
                cr.move_to(center-half,f64::from(h)/2.0);cr.line_to(center+half+0.01,f64::from(h)/2.0);
                let _=cr.stroke();
            }
        });
        root.append(&row);root.append(&area);root.set_visible(!buttons.is_empty());
        Self{root,buttons,marks,area,running:Rc::new(Cell::new(false)),local_reduced:Rc::new(Cell::new(false))}
    }
    pub fn update(&self,snapshot:Option<&Snapshot>,local_reduced:bool){
        self.local_reduced.set(local_reduced);
        let changed=self.marks.borrow().confirmed!=snapshot.map(|s|s.active);
        let snap=reduced(local_reduced);
        if changed||snap{
            self.marks.borrow_mut().update(snapshot,snap);
            self.area.queue_draw();
        }
        if changed{for (index,button) in self.buttons.iter().enumerate(){
            let active=snapshot.is_some_and(|s|s.active[index]);
            button.set_sensitive(snapshot.is_some());
            if active{button.add_css_class("suggested-action");}else{button.remove_css_class("suggested-action");}
            let description=format!("Workspace {}{}",index+1,if snapshot.is_none(){" · Unavailable"}else if active{" · Active"}else{""});
            button.set_tooltip_text(Some(&description));
            button.update_property(&[gtk::accessible::Property::Label(&description)]);
        }
        }
        if !self.marks.borrow().moving()||self.running.replace(true){return;}
        let (marks,running,local)=(self.marks.clone(),self.running.clone(),self.local_reduced.clone());
        let last=Cell::new(None::<i64>);
        self.area.add_tick_callback(move|area,clock|{
            let now=clock.frame_time();
            let dt=last.replace(Some(now)).map_or(0.0,|old|now.saturating_sub(old) as f64/1_000_000.0);
            let moving=marks.borrow_mut().advance(dt,reduced(local.get()));
            area.queue_draw();
            if moving{glib::ControlFlow::Continue}else{running.set(false);glib::ControlFlow::Break}
        });
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    fn state(indices:&[usize])->Snapshot{
        let mut active=[false;9];for &i in indices{active[i]=true;}Snapshot{active}
    }
    #[test]
    fn multiple_active_tags_and_unavailable_reset(){
        let mut m=Marks::new(9);m.update(Some(&state(&[1,4])),false);
        assert_eq!(m.width(1,40.0),26.0);assert_eq!(m.width(4,40.0),26.0);assert_eq!(m.width(0,40.0),0.0);
        m.update(Some(&state(&[2,4])),false);assert!(m.moving());m.advance(0.03,false);
        m.update(None,false);assert!(!m.moving());assert_eq!(m.width(4,40.0),0.0);
        m.update(Some(&state(&[7])),false);assert!(!m.moving());assert_eq!(m.width(7,40.0),26.0);
    }
    #[test]
    fn rapid_retargets_remain_inside_cells_and_reduce_motion_snaps(){
        let mut m=Marks::new(4);m.update(Some(&state(&[0])),false);
        for i in [1,3,0,2]{
            m.update(Some(&state(&[i])),false);
            for _ in 0..3{
                m.advance(0.016,false);
                for j in 0..4{assert!((0.0..=14.0).contains(&m.width(j,20.0)));}
            }
        }
        m.update(Some(&state(&[2])),true);assert!(!m.moving());
        assert_eq!(m.width(2,40.0),26.0);assert_eq!(m.width(0,40.0),0.0);
    }
}
