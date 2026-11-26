//! Built-in and configurable script presets; never execute hooks.
use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Clone,Debug,PartialEq,Eq,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct Preset {
    pub icon: String,
    pub urgency: u8,
    pub timeout_ms: i32,
}
impl Default for Preset{
    fn default()->Self{Self{icon:"dialog-information-symbolic".into(),urgency:1,timeout_ms:5000}}
}
impl Preset{
    pub fn validate(&self)->Result<()>{
        if self.icon.is_empty()||self.icon.len()>128||!self.icon.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'-'|b'_')){
            bail!("preset icon must be a simple theme icon name");
        }
        if self.urgency>2{bail!("preset urgency must be 0, 1 or 2");}
        if self.timeout_ms!=0 && !(1000..=60000).contains(&self.timeout_ms){bail!("preset timeout must be 0 (persistent) or 1000–60000 ms");}
        Ok(())
    }
}
pub fn builtin(name:&str)->Option<Preset>{
    let (icon,urgency,timeout_ms)=match name{
        "info"=>("dialog-information-symbolic",1,5000),
        "success"=>("emblem-ok-symbolic",1,5000),
        "warning"=>("dialog-warning-symbolic",1,8000),
        "error"=>("dialog-error-symbolic",2,0),
        "download"=>("folder-download-symbolic",1,5000),
        "update"=>("system-software-update-symbolic",1,5000),
        _=>return None,
    };
    Some(Preset{icon:icon.into(),urgency,timeout_ms})
}
pub fn parse_duration(value:&str)->Result<u32>{
    if value=="off"{return Ok(0);}
    let (digits,multiplier)=match value.as_bytes().last(){
        Some(b's')=>(&value[..value.len()-1],1u32),
        Some(b'm')=>(&value[..value.len()-1],60),
        Some(b'h')=>(&value[..value.len()-1],3600),
        _=>bail!("use a duration with s, m or h, or off"),
    };
    if digits.is_empty()||!digits.bytes().all(|b|b.is_ascii_digit()){bail!("invalid duration");}
    let seconds=digits.parse::<u32>().context("invalid duration")?.checked_mul(multiplier).context("duration overflow")?;
    if !(1..=86400).contains(&seconds){bail!("pause must be between 1 second and 24 hours");}
    Ok(seconds)
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn durations_are_bounded_and_explicit(){
        for (input,value) in [("off",0),("1s",1),("30m",1800),("24h",86400)]{assert_eq!(parse_duration(input).unwrap(),value);}
        for input in ["0s","25h","1.5h","-1m","30","4294967295h","x","１m"]{assert!(parse_duration(input).is_err());}
    }
    #[test]
    fn presets_are_fixed_and_do_not_execute(){
        assert_eq!(builtin("success").unwrap().icon,"emblem-ok-symbolic");
        assert_eq!(builtin("error").unwrap().urgency,2);
        assert!(builtin("$(id)").is_none());
    }
}
