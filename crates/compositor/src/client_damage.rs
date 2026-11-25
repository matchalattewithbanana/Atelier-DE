//! Experimental conservative client damage on retained native buffers.
//! Never assumes shader effects or a changed stacking/geometry are local.
use crate::scene::SceneElement;
use smithay::{
    backend::renderer::{element::{Element,Id,surface::WaylandSurfaceTexture},utils::CommitCounter},
    utils::{Buffer,Physical,Rectangle,Scale,Transform},
};

const MAX_SURFACES:usize=256;
const MAX_DAMAGE_RECTS:usize=64;

#[derive(Clone)]
struct Surface {
    id:Id,
    commit:CommitCounter,
    geometry:Rectangle<i32,Physical>,
    source:Rectangle<f64,Buffer>,
    transform:Transform,
    alpha:f32,
}

impl Surface {
    fn same_placement(&self,other:&Self)->bool{
        self.id==other.id&&self.geometry==other.geometry&&self.source==other.source
            &&self.transform==other.transform&&self.alpha==other.alpha
    }
}

pub struct Snapshot {
    surfaces:Vec<Surface>,
    cursor:Rectangle<i32,Physical>,
}

fn union(a:Rectangle<i32,Physical>,b:Rectangle<i32,Physical>)->Rectangle<i32,Physical>{
    // Both inputs are clipped to the bounded output.
    let l=a.loc.x.min(b.loc.x);let t=a.loc.y.min(b.loc.y);
    let r=(a.loc.x+a.size.w).max(b.loc.x+b.size.w);
    let bottom=(a.loc.y+a.size.h).max(b.loc.y+b.size.h);
    Rectangle::new((l,t).into(),(r-l,bottom-t).into())
}

/// Convert element-local physical damage to one clipped output rectangle.
/// Expand by a pixel for filtering; invalid, empty or oversized histories
/// fall back to whole-surface damage instead of assuming nothing changed.
fn damage_bounds(rects:&[Rectangle<i32,Physical>],geometry:Rectangle<i32,Physical>,output:Rectangle<i32,Physical>)->Option<Rectangle<i32,Physical>>{
    if rects.is_empty()||rects.len()>MAX_DAMAGE_RECTS{return None;}
    let mut result=None;
    for rect in rects{
        if rect.size.w<=0||rect.size.h<=0{return None;}
        let x=i64::from(rect.loc.x);let y=i64::from(rect.loc.y);
        let w=i64::from(geometry.size.w);let h=i64::from(geometry.size.h);
        let r=x+i64::from(rect.size.w);let b=y+i64::from(rect.size.h);
        if x<0||y<0||r>w||b>h{return None;}
        let gx=i64::from(geometry.loc.x);let gy=i64::from(geometry.loc.y);
        let l=gx+(x-1).max(0);let t=gy+(y-1).max(0);
        let r=gx+(r+1).min(w);let b=gy+(b+1).min(h);
        let ox=i64::from(output.loc.x);let oy=i64::from(output.loc.y);
        let l=l.max(ox);let t=t.max(oy);
        let r=r.min(ox+i64::from(output.size.w));let b=b.min(oy+i64::from(output.size.h));
        if l>=r||t>=b{continue;}
        let visible=Rectangle::new((i32::try_from(l).ok()?,i32::try_from(t).ok()?).into(),
            (i32::try_from(r-l).ok()?,i32::try_from(b-t).ok()?).into());
        result=Some(result.map_or(visible,|old|union(old,visible)));
    }
    result
}

// All geometry, alpha, source mapping, identity and stacking must match.
// A changed commit uses the element's own physical damage history where
// supported, otherwise its entire visible extent. Each cursor belongs
// to the same retained scanout slot as the corresponding commit counters.
fn compare(old:&Snapshot,new:&Snapshot,output:Rectangle<i32,Physical>,
    mut changed:impl FnMut(usize,&Surface,&Surface)->Option<Rectangle<i32,Physical>>)->Rectangle<i32,Physical>{
    if old.surfaces.len()!=new.surfaces.len(){return output;}
    let mut damage=union(old.cursor,new.cursor);
    for (index,(a,b)) in old.surfaces.iter().zip(&new.surfaces).enumerate(){
        if !a.same_placement(b){return output;}
        if a.commit!=b.commit{
            if let Some(visible)=changed(index,a,b).or_else(||b.geometry.intersection(output)){damage=union(damage,visible);}
        }
    }
    damage.intersection(output).unwrap_or(output)
}

pub struct ClientDamage {
    enabled:bool,
    slots:[Option<Snapshot>;2],
}
impl ClientDamage {
    pub fn new()->Self{
        Self{enabled:std::env::var("ATELIER_CLIENT_DAMAGE").as_deref()==Ok("1"),slots:[None,None]}
    }

    pub fn reset(&mut self){self.slots=[None,None];}

    pub fn capture(&self,elements:&[SceneElement],cursor:Rectangle<i32,Physical>)->Option<Snapshot>{
        if !self.enabled||elements.len()>MAX_SURFACES{return None;}
        let mut surfaces=Vec::with_capacity(elements.len());
        for element in elements{
            // Shader decorations, blur and rounded masks have additional
            // dependencies not described by the ordinary surface contract.
            let SceneElement::Surface(element)=element else{return None;};
            let id=element.id().clone();
            if surfaces.iter().any(|s:&Surface|s.id==id){return None;}
            surfaces.push(Surface{id,commit:element.current_commit(),geometry:element.geometry(Scale::from(1.0)),
                source:element.src(),transform:element.transform(),alpha:element.alpha()});
        }
        Some(Snapshot{surfaces,cursor})
    }

    pub fn damage(&self,slot:usize,output:Rectangle<i32,Physical>,fallback:Rectangle<i32,Physical>,next:Option<&Snapshot>,elements:&[SceneElement])->Rectangle<i32,Physical>{
        if !self.enabled{return fallback;}
        match (&self.slots[slot],next){
            (Some(old),Some(next))=>compare(old,next,output,|index,a,b|{
                let SceneElement::Surface(element)=elements.get(index)? else{return None;};
                // Start with untransformed, 1:1 textured sampling only.
                // Viewport scaling, rotated buffers and solid-color protocol
                // buffers keep whole-surface repaint until visual acceptance.
                if b.transform!=Transform::Normal||b.source.loc.x.fract()!=0.0||b.source.loc.y.fract()!=0.0
                    ||b.source.size.w!=f64::from(b.geometry.size.w)||b.source.size.h!=f64::from(b.geometry.size.h)
                    ||!matches!(element.texture(),WaylandSurfaceTexture::Texture(_)){return None;}
                let damage=element.damage_since(Scale::from(1.0),Some(a.commit));
                damage_bounds(&damage,b.geometry,output)
            }),
            _=>output,
        }
    }

    // A failed draw/submission never becomes the baseline for retained pixels.
    pub fn submitted(&mut self,slot:usize,snapshot:Option<Snapshot>){self.slots[slot]=snapshot;}
}

#[cfg(test)]
mod tests{
    use super::*;
    fn rect(x:i32,y:i32,w:i32,h:i32)->Rectangle<i32,Physical>{Rectangle::new((x,y).into(),(w,h).into())}
    fn surface()->Surface{Surface{id:Id::new(),commit:CommitCounter::default(),geometry:rect(100,100,200,100),
        source:Rectangle::from_size((200.0,100.0).into()),transform:Transform::Normal,alpha:1.0}}
    #[test]
    fn changed_commit_repaints_surface_and_both_cursor_positions(){
        let output=rect(0,0,1000,800);
        let surface=surface();
        let old=Snapshot{surfaces:vec![surface.clone()],cursor:rect(10,10,11,16)};
        let mut new=Snapshot{surfaces:vec![surface],cursor:rect(20,20,11,16)};
        assert_eq!(compare(&old,&new,output,|_,_,_|None),rect(10,10,21,26));
        new.surfaces[0].commit.increment();
        assert_eq!(compare(&old,&new,output,|_,_,_|None),rect(10,10,290,190));
        new.surfaces[0].alpha=0.8;
        assert_eq!(compare(&old,&new,output,|_,_,_|None),output);
        new.surfaces.clear();
        assert_eq!(compare(&old,&new,output,|_,_,_|None),output);
    }
    #[test]
    fn local_history_is_translated_padded_and_clipped(){
        let output=rect(0,0,1000,800);let geom=rect(100,100,200,100);
        assert_eq!(damage_bounds(&[rect(10,20,5,5)],geom,output),Some(rect(109,119,7,7)));
        assert_eq!(damage_bounds(&[rect(0,0,200,100)],geom,output),Some(geom));
        assert_eq!(damage_bounds(&[rect(0,0,5,5)],rect(-2,-2,200,100),output),Some(rect(0,0,4,4)));
        assert!(damage_bounds(&[],geom,output).is_none());
        assert!(damage_bounds(&[rect(199,0,2,1)],geom,output).is_none());
        assert!(damage_bounds(&vec![rect(0,0,1,1);MAX_DAMAGE_RECTS+1],geom,output).is_none());
        let old=Snapshot{surfaces:vec![surface()],cursor:rect(100,100,11,16)};
        let mut new=Snapshot{surfaces:old.surfaces.clone(),cursor:old.cursor};
        new.surfaces[0].commit.increment();
        let local=compare(&old,&new,output,|_,_,b|damage_bounds(&[rect(10,20,5,5)],b.geometry,output));
        assert_eq!(local,rect(100,100,16,26));
        assert_eq!(compare(&old,&new,output,|_,_,_|None),geom);
    }
    #[test]
    fn slot_baselines_are_independent_and_reset_is_conservative(){
        let output=rect(0,0,1000,800);let cursor=rect(10,10,11,16);
        let mut history=ClientDamage{enabled:true,slots:[None,None]};
        let snapshot=Snapshot{surfaces:vec![],cursor};
        assert_eq!(history.damage(0,output,cursor,Some(&snapshot),&[]),output);
        history.submitted(0,Some(Snapshot{surfaces:vec![],cursor}));
        assert_eq!(history.damage(0,output,cursor,Some(&snapshot),&[]),cursor);
        assert_eq!(history.damage(1,output,cursor,Some(&snapshot),&[]),output);
        assert_eq!(history.damage(0,output,cursor,None,&[]),output);
        history.reset();
        assert_eq!(history.damage(0,output,cursor,Some(&snapshot),&[]),output);
        history.enabled=false;
        assert_eq!(history.damage(0,output,cursor,None,&[]),cursor);
    }
}
