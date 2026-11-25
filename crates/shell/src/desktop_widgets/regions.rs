//! Publish the visible card input shape using the standard Wayland surface
//! input region. This is also a native compositor blur hint. Scrollbar
//! input is retained; transparent gaps are deliberately click-through.
use gtk::{glib,prelude::*};
type Rect=[i32;4]; // x, y, width, height
fn intersect(a:Rect,b:Rect)->Option<Rect>{
    let x=a[0].max(b[0]);let y=a[1].max(b[1]);
    let r=a[0].saturating_add(a[2]).min(b[0].saturating_add(b[2]));
    let bottom=a[1].saturating_add(a[3]).min(b[1].saturating_add(b[3]));
    (r>x&&bottom>y).then(||[x,y,r-x,bottom-y])
}
/// Rounded pixel-centre scanlines, clipped after computing the original
/// card shape so partially scrolled cards do not gain new rounded edges.
fn rounded(rect:Rect,radius:u32,clip:Rect)->Vec<Rect>{
    let [x,y,w,h]=rect;
    if w<=0||h<=0{return Vec::new();}
    let r=(radius.min(24) as i32).min(w/2).min(h/2);
    if r==0{return intersect(rect,clip).into_iter().collect();}
    let mut rows=Vec::with_capacity((r*2+1) as usize);
    for row in 0..r{
        let dy=f64::from(r-row)-0.5;
        let inset=(f64::from(r)-(f64::from(r*r)-dy*dy).sqrt()-0.5).ceil().max(0.0) as i32;
        for offset in [row,h-1-row]{
            if let Some(hit)=intersect([x+inset,y+offset,w-2*inset,1],clip){rows.push(hit);}
        }
    }
    if let Some(hit)=intersect([x,y+r,w,h-2*r],clip){rows.push(hit);}
    rows
}
fn bounds(widget:&impl IsA<gtk::Widget>,window:&gtk::ApplicationWindow)->Option<Rect>{
    if !widget.is_mapped(){return None;}
    let b=widget.compute_bounds(window)?;
    let (x,y,r,bottom)=(b.x().floor() as i32,b.y().floor() as i32,(b.x()+b.width()).ceil() as i32,(b.y()+b.height()).ceil() as i32);
    (r>x&&bottom>y).then(||[x,y,r-x,bottom-y])
}
pub(super) struct RegionPublisher{
    window:glib::WeakRef<gtk::ApplicationWindow>,
    scroll:glib::WeakRef<gtk::ScrolledWindow>,
    group:glib::WeakRef<gtk::Box>,
    last:Option<Vec<Rect>>,
}
impl RegionPublisher{
    pub fn new(window:&gtk::ApplicationWindow,scroll:&gtk::ScrolledWindow,group:&gtk::Box)->Self{
        Self{window:window.downgrade(),scroll:scroll.downgrade(),group:group.downgrade(),last:None}
    }
    pub fn publish(&mut self,radius:u32){
        let (Some(window),Some(scroll),Some(group))=(self.window.upgrade(),self.scroll.upgrade(),self.group.upgrade())else{return;};
        let Some(surface)=window.surface()else{self.last=None;return;};
        if !window.is_mapped(){self.last=None;return;}
        let area=[0,0,window.width(),window.height()];
        let viewport=scroll.child().and_then(|child|bounds(&child,&window)).and_then(|r|intersect(r,area));
        let mut rows=Vec::new();
        if let Some(viewport)=viewport{
            let mut child=group.first_child();
            while let Some(card)=child{
                child=card.next_sibling();
                if let Some(rect)=bounds(&card,&window){rows.extend(rounded(rect,radius,viewport));}
            }
            let scrollbar=scroll.vscrollbar();
            if let Some(rect)=bounds(&scrollbar,&window).and_then(|r|intersect(r,area)){rows.push(rect);}
        }
        // If allocations disappear during a layout change or exceed the
        // protocol budget, clear the old region instead of intercepting
        // clicks in stale card locations. The next valid tick restores it.
        if rows.len()>256{rows.clear();}
        if self.last.as_ref()==Some(&rows){return;}
        let region=gtk::cairo::Region::create();
        for [x,y,w,h] in &rows{
            if region.union_rectangle(&gtk::cairo::RectangleInt::new(*x,*y,*w,*h)).is_err(){return;}
        }
        // GDK stages Wayland input regions until the next surface commit.
        // A static card may not otherwise schedule a render for minutes.
        surface.set_input_region(&region);
        window.queue_draw();
        surface.queue_render();
        if self.last.as_ref().is_none_or(Vec::is_empty)&&!rows.is_empty(){println!("Atelier widget card input region published");}
        self.last=Some(rows);
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn scroll_clipping_preserves_original_corner_shape(){
        let card=[0,-10,100,40];let viewport=[0,0,100,100];
        let rows=rounded(card,12,viewport);
        assert!(rows.iter().any(|r|r[0]==0&&r[1]<=2));
        assert!(rows.iter().all(|r|r[1]>=0&&r[1]+r[3]<=30));
        assert!(!rows.iter().any(|r|r[1]==29&&r[0]==0));
    }
    #[test]
    fn gaps_and_corner_pixels_remain_outside(){
        let viewport=[0,0,100,100];
        let rows=[rounded([0,0,100,40],12,viewport),rounded([0,48,100,40],12,viewport)].concat();
        assert!(!rows.iter().any(|r|r[1]<=44&&44<r[1]+r[3]));
        assert!(!rows.iter().any(|r|r[0]==0&&r[1]==0));
        assert!(rows.len()<=50);
    }
}
