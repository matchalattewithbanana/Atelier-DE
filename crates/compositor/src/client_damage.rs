//! Experimental conservative whole-surface damage on retained native buffers.
//! Never assumes shader effects or a changed stacking/geometry are local.
use crate::scene::SceneElement;
use smithay::{
    backend::renderer::{element::{Element,Id},utils::CommitCounter},
    utils::{Buffer,Physical,Rectangle,Scale,Transform},
};

const MAX_SURFACES:usize=256;

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

// All geometry, alpha, source mapping, identity and stacking must match.
// Repaint the entire visible extent of any element with a changed commit,
// rather than trusting buffer-space subrectangles. Cursor old/new extents
// are from the same scanout slot, not from the most recently displayed one.
fn compare(old:&Snapshot,new:&Snapshot,output:Rectangle<i32,Physical>)->Rectangle<i32,Physical>{
    if old.surfaces.len()!=new.surfaces.len(){return output;}
    let mut damage=union(old.cursor,new.cursor);
    for (a,b) in old.surfaces.iter().zip(&new.surfaces){
        if !a.same_placement(b){return output;}
        if a.commit!=b.commit{
            if let Some(visible)=b.geometry.intersection(output){damage=union(damage,visible);}
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

    pub fn damage(&self,slot:usize,output:Rectangle<i32,Physical>,fallback:Rectangle<i32,Physical>,next:Option<&Snapshot>)->Rectangle<i32,Physical>{
        if !self.enabled{return fallback;}
        match (&self.slots[slot],next){
            (Some(old),Some(next))=>compare(old,next,output),
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
        assert_eq!(compare(&old,&new,output),rect(10,10,21,26));
        new.surfaces[0].commit.increment();
        assert_eq!(compare(&old,&new,output),rect(10,10,290,190));
        new.surfaces[0].alpha=0.8;
        assert_eq!(compare(&old,&new,output),output);
        new.surfaces.clear();
        assert_eq!(compare(&old,&new,output),output);
    }
    #[test]
    fn slot_baselines_are_independent_and_reset_is_conservative(){
        let output=rect(0,0,1000,800);let cursor=rect(10,10,11,16);
        let mut history=ClientDamage{enabled:true,slots:[None,None]};
        let snapshot=Snapshot{surfaces:vec![],cursor};
        assert_eq!(history.damage(0,output,cursor,Some(&snapshot)),output);
        history.submitted(0,Some(Snapshot{surfaces:vec![],cursor}));
        assert_eq!(history.damage(0,output,cursor,Some(&snapshot)),cursor);
        assert_eq!(history.damage(1,output,cursor,Some(&snapshot)),output);
        assert_eq!(history.damage(0,output,cursor,None),output);
        history.reset();
        assert_eq!(history.damage(0,output,cursor,Some(&snapshot)),output);
        history.enabled=false;
        assert_eq!(history.damage(0,output,cursor,None),cursor);
    }
}
