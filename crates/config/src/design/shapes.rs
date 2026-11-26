//! Native expressive silhouettes in a common radial representation.
//! These are Atelier originals, not exact Material shape asset paths.
//! Equal-angle correspondence keeps intermediate shapes simple and bounded.
use std::f64::consts::{FRAC_PI_2,TAU};
const SAMPLES:usize=120;
type Point=(f64,f64);
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum Shape{Circle,Pentagon,Slanted,Gem,SoftBurst}
impl Shape{
    fn vertices(self)->Option<&'static [Point]>{
        match self{
            Self::Pentagon=>Some(&[(0.50,0.04),(0.96,0.37),(0.79,0.94),(0.21,0.94),(0.04,0.37)]),
            Self::Slanted=>Some(&[(0.20,0.05),(0.95,0.05),(0.80,0.95),(0.05,0.95)]),
            Self::Gem=>Some(&[(0.27,0.04),(0.73,0.04),(0.97,0.39),(0.50,0.96),(0.03,0.39)]),
            _=>None,
        }
    }
    pub fn outline(self)->Outline{
        let raw=std::array::from_fn(|i|{
            let angle=angle(i);
            if let Some(vertices)=self.vertices(){
                let direction=(angle.cos(),angle.sin());
                let mut distance=f64::INFINITY;
                for (i,&p) in vertices.iter().enumerate(){
                    let q=vertices[(i+1)%vertices.len()];
                    let edge=(q.0-p.0,q.1-p.1);
                    let offset=(p.0-0.5,p.1-0.5);
                    let denom=cross(direction,edge);
                    if denom.abs()<1e-12{continue;}
                    let t=cross(offset,edge)/denom;
                    let u=cross(offset,direction)/denom;
                    if t>=0.0&&(0.0..=1.0).contains(&u){distance=distance.min(t);}
                }
                distance
            }else if self==Self::SoftBurst{
                0.405+0.063*(12.0*angle).cos()
            }else{0.46}
        });
        // Smooth polygon corners in angular space while retaining their
        // silhouette. A symmetric kernel also preserves the wrap seam.
        let radii=if self.vertices().is_some(){
            std::array::from_fn(|i|{
                [-2isize,-1,0,1,2].into_iter().zip([1.,4.,6.,4.,1.]).map(|(d,w)|{
                    raw[(i as isize+d).rem_euclid(SAMPLES as isize) as usize]*w
                }).sum::<f64>()/16.0
            })
        }else{raw};
        Outline{radii}
    }
}
fn cross(a:Point,b:Point)->f64{a.0*b.1-a.1*b.0}
fn angle(i:usize)->f64{TAU*i as f64/SAMPLES as f64-FRAC_PI_2}

#[derive(Debug,Clone,PartialEq)]
pub struct Outline{radii:[f64;SAMPLES]}
impl Outline{
    /// Morph two silhouettes without topology changes. Clamp to safe
    /// geometry: spatial spring overshoot must not distort an icon mask.
    pub fn interpolate(&self,to:&Self,progress:f64)->Self{
        let t=if progress.is_finite(){progress.clamp(0.0,1.0)}else{0.0};
        if t==0.0{return self.clone();}
        if t==1.0{return to.clone();}
        Self{radii:std::array::from_fn(|i|self.radii[i]*(1.0-t)+to.radii[i]*t)}
    }
    /// Closed unit-square path; renderer closes the final edge.
    pub fn points(&self)->impl Iterator<Item=Point>+'_{
        self.radii.iter().enumerate().map(|(i,r)|{
            let a=angle(i);
            // Smoothing can protrude fractionally beyond a polygon edge.
            ((0.5+r*a.cos()).clamp(0.0,1.0),(0.5+r*a.sin()).clamp(0.0,1.0))
        })
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn all_morphs_are_finite_bounded_and_ordered(){
        let shapes=[Shape::Circle,Shape::Pentagon,Shape::Slanted,Shape::Gem,Shape::SoftBurst];
        for a in shapes{
            for b in shapes{
                let from=a.outline();let to=b.outline();
                assert_eq!(from.interpolate(&to,-1.0),from);
                assert_eq!(from.interpolate(&to,2.0),to);
                for t in [0.,0.25,0.5,0.75,1.]{
                    let path=from.interpolate(&to,t);
                    let points:Vec<_>=path.points().collect();
                    assert_eq!(points.len(),SAMPLES);
                    for (i,p) in points.iter().enumerate(){
                        assert!((0.0..=1.0).contains(&p.0)&&(0.0..=1.0).contains(&p.1));
                        let next=points[(i+1)%SAMPLES];
                        assert!(cross((p.0-0.5,p.1-0.5),(next.0-0.5,next.1-0.5))>0.0);
                    }
                }
            }
        }
    }
}
