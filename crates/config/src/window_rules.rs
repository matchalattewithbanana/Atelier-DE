//! Machine-local appearance exceptions, keyed by exact xdg-shell app ID.
//! No matching against executable paths, titles, patterns or credentials.
use anyhow::{bail,Result};
use serde::{Deserialize,Serialize};
use std::collections::BTreeMap;
use crate::Transparency;

#[derive(Debug,Default,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct WindowRule{
    pub focused_opacity:Option<u32>,
    pub inactive_opacity:Option<u32>,
    pub corner_radius:Option<u32>,
    pub backdrop_blur:Option<u32>,
    pub disable_shadow:bool,
}
impl WindowRule{
    fn validate(&self)->Result<()>{
        if [self.focused_opacity,self.inactive_opacity].into_iter().flatten().any(|v|!(70..=100).contains(&v)){bail!("opacity overrides must be 70–100%");}
        if self.corner_radius.is_some_and(|v|v>24){bail!("corner override must be 0–24");}
        if self.backdrop_blur.is_some_and(|v|v>16){bail!("blur override must be 0–16");}
        if *self==Self::default(){bail!("a rule must override at least one setting; remove it to inherit everything");}
        Ok(())
    }
    pub fn alpha(self,defaults:Transparency,focused:bool)->f32{
        let opacity=if focused{self.focused_opacity.unwrap_or(defaults.focused)}else{self.inactive_opacity.unwrap_or(defaults.unfocused)};
        opacity as f32/100.0
    }
}
#[derive(Debug,Default,Clone,PartialEq,Eq,Serialize,Deserialize)]
#[serde(transparent)]
pub struct WindowRules(BTreeMap<String,WindowRule>);
impl WindowRules{
    fn validate_id(id:&str)->Result<()>{
        if id.is_empty()||id.len()>128||id.trim()!=id||id.chars().any(|c|c.is_control()||c.is_whitespace()||matches!(c,'/'|'\\'|'*'|'?'|'['|']')){bail!("use an exact nonempty xdg app ID, at most 128 bytes, without whitespace, paths or patterns");}
        Ok(())
    }
    pub fn validate(&self)->Result<()>{
        if self.0.len()>64{bail!("at most 64 application appearance rules");}
        for (id,rule) in &self.0{Self::validate_id(id)?;rule.validate()?;}
        Ok(())
    }
    pub fn get(&self,id:&str)->Option<&WindowRule>{self.0.get(id)}
    pub fn ids(&self)->impl Iterator<Item=&str>{self.0.keys().map(String::as_str)}
    pub fn is_empty(&self)->bool{self.0.is_empty()}
    pub fn insert(&mut self,id:String,rule:WindowRule)->Result<()>{
        Self::validate_id(&id)?;rule.validate()?;
        if self.0.len()>=64&&!self.0.contains_key(&id){bail!("at most 64 application appearance rules");}
        self.0.insert(id,rule);Ok(())
    }
    pub fn remove(&mut self,id:&str)->bool{self.0.remove(id).is_some()}
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn exact_matching_and_independent_inheritance(){
        let mut rules=WindowRules::default();
        let rule=WindowRule{focused_opacity:Some(100),corner_radius:Some(0),disable_shadow:true,..WindowRule::default()};
        rules.insert("org.example.Video".into(),rule).unwrap();
        assert_eq!(rules.get("org.example.Video"),Some(&rule));
        assert!(rules.get("org.example.video").is_none());
        assert!(rules.get("org.example.Video.desktop").is_none());
        assert!(rules.get("").is_none());
        let defaults=Transparency{focused:80,unfocused:90,..Transparency::default()};
        assert_eq!(rule.alpha(defaults,true),1.0);
        assert_eq!(rule.alpha(defaults,false),0.9);
        assert!(rules.remove("org.example.Video"));
        assert!(rules.is_empty());
    }
    #[test]
    fn reject_invalid_and_bound_rule_count(){
        let rule=WindowRule{backdrop_blur:Some(0),..WindowRule::default()};
        let mut rules=WindowRules::default();
        for id in ["","*","org.*","/bin/foo","with space","x\n"]{assert!(rules.insert(id.into(),rule).is_err());}
        assert!(rules.insert("app".into(),WindowRule::default()).is_err());
        assert!(rules.insert("app".into(),WindowRule{focused_opacity:Some(69),..rule}).is_err());
        assert!(rules.insert("app".into(),WindowRule{corner_radius:Some(25),..rule}).is_err());
        assert!(rules.insert("app".into(),WindowRule{backdrop_blur:Some(17),..rule}).is_err());
        for i in 0..64{rules.insert(format!("app{i}"),rule).unwrap();}
        assert!(rules.insert("new".into(),rule).is_err());
        assert!(rules.insert("app0".into(),rule).is_ok());
        assert!(rules.validate().is_ok());
    }
}
