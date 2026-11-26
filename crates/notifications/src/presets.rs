//! Built-in script presets; no commands, network access or executable hooks.
use anyhow::{bail, Context, Result};

pub struct Preset {
    pub icon: &'static str,
    pub urgency: u8,
    pub timeout_ms: i32,
}
pub fn resolve(name:&str)->Result<Preset>{
    let (icon,urgency,timeout_ms)=match name{
        "info"=>("dialog-information-symbolic",1,5000),
        "success"=>("emblem-ok-symbolic",1,5000),
        "warning"=>("dialog-warning-symbolic",1,8000),
        "error"=>("dialog-error-symbolic",2,0),
        "download"=>("folder-download-symbolic",1,5000),
        "update"=>("system-software-update-symbolic",1,5000),
        _=>bail!("unknown preset: choose info, success, warning, error, download or update"),
    };
    Ok(Preset{icon,urgency,timeout_ms})
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
        assert_eq!(resolve("success").unwrap().icon,"emblem-ok-symbolic");
        assert_eq!(resolve("error").unwrap().urgency,2);
        assert!(resolve("$(id)").is_err());
    }
}
