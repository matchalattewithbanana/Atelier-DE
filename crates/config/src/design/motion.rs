//! Retargetable normalized spring values for toolkit frame clocks.
use super::Spring;

#[derive(Debug,Clone,Copy)]
pub struct SpringValue{
    spring:Spring,value:f64,target:f64,velocity:f64,
}
impl SpringValue{
    pub fn new(spring:Spring,value:f64)->Self{
        let value=unit(value);
        Self{spring,value,target:value,velocity:0.0}
    }
    pub fn value(&self)->f64{self.value}
    pub fn settled(&self)->bool{self.value==self.target&&self.velocity==0.0}
    /// Retarget from the current position. Spatial velocity survives reversal;
    /// effects reset velocity deliberately so opacity/color cannot overshoot.
    pub fn retarget(&mut self,target:f64){
        let target=unit(target);
        if target==self.target{return;}
        self.target=target;
        if self.spring.damping==1.0{self.velocity=0.0;}
    }
    pub fn advance(&mut self,seconds:f64,reduced_motion:bool)->f64{
        if reduced_motion||seconds>=4.0{
            self.value=self.target;self.velocity=0.0;return self.value;
        }
        if !seconds.is_finite()||seconds<=0.0||self.settled(){return self.value;}
        let omega=self.spring.stiffness.sqrt();
        let z=self.spring.damping;
        let y=self.value-self.target;let v=self.velocity;
        let a=z*omega;let decay=(-a*seconds).exp();
        let (next,velocity)=if z==1.0{
            let b=v+omega*y;
            let p=y+b*seconds;
            (decay*p,decay*(b-omega*p))
        }else{
            let w=omega*(1.0-z*z).sqrt();
            let b=(v+a*y)/w;
            let (sin,cos)=(w*seconds).sin_cos();
            let p=y*cos+b*sin;
            (decay*p,decay*(-a*p+w*(-y*sin+b*cos)))
        };
        let value=self.target+next;
        // Numerical and retarget safety for normalized properties.
        self.value=if z==1.0{value.clamp(self.value.min(self.target),self.value.max(self.target))}else{value};
        self.velocity=velocity;
        if (self.value-self.target).abs()<0.0001&&self.velocity.abs()<0.001{
            self.value=self.target;self.velocity=0.0;
        }
        self.value
    }
}
fn unit(value:f64)->f64{if value.is_finite(){value.clamp(0.0,1.0)}else{0.0}}

#[cfg(test)]
mod tests{
    use super::*;
    use crate::design::{Scheme,Speed,MotionKind};
    #[test]
    fn spatial_retarget_preserves_position_and_velocity(){
        let mut s=SpringValue::new(Scheme::Expressive.spring(Speed::Fast,MotionKind::Spatial),0.0);
        s.retarget(1.0);s.advance(0.03,false);
        let (value,velocity)=(s.value,s.velocity);
        s.retarget(0.0);assert_eq!((s.value,s.velocity),(value,velocity));
        for _ in 0..240{s.advance(1.0/120.0,false);}
        assert!(s.settled());assert_eq!(s.value(),0.0);
    }
    #[test]
    fn effects_never_overshoot_on_reversal(){
        let mut s=SpringValue::new(Scheme::Expressive.spring(Speed::Default,MotionKind::Effects),0.0);
        for target in [1.0,0.0,0.7,0.1]{
            s.retarget(target);
            for _ in 0..8{
                let before=s.value();let next=s.advance(0.016,false);
                assert!(next>=before.min(target)&&next<=before.max(target));
            }
        }
        assert_eq!(s.advance(0.0,true),0.1);assert!(s.settled());
    }
    #[test]
    fn integration_agrees_with_analytic_sampler(){
        let spring=Scheme::Expressive.spring(Speed::Default,MotionKind::Spatial);
        let mut s=SpringValue::new(spring,0.0);s.retarget(1.0);
        for i in 1..=20{
            let value=s.advance(0.01,false);
            assert!((value-spring.progress(f64::from(i)*0.01,false)).abs()<1e-9);
        }
    }
}
