//! Pause duration parsing for notification CLI.
use anyhow::{bail, Context, Result};

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
}
