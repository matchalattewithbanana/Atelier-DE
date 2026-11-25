//! Linux system summary; load average is not CPU utilization.
use std::{fs,io,path::Path};
#[derive(Debug,PartialEq)]
pub struct SystemSnapshot {
    pub load_1m:Option<f64>,
    pub memory_total_kib:Option<u64>,
    pub memory_used_kib:Option<u64>,
}
fn meminfo(text:&str)->(Option<u64>,Option<u64>){
    let field=|key:&str|text.lines().find_map(|line|{
        let (name,value)=line.split_once(':')?;
        if name!=key{return None;}
        let mut fields=value.split_whitespace();let n=fields.next()?.parse::<u64>().ok()?;
        (fields.next()==Some("kB")).then_some(n)
    });
    let total=field("MemTotal").filter(|n|*n>0);
    let used=total.zip(field("MemAvailable")).and_then(|(total,available)|total.checked_sub(available));
    (total,used)
}
fn read(root:&Path,name:&str)->io::Result<String>{
    use std::io::Read;
    let mut bytes=String::new();
    fs::File::open(root.join(name))?.take(65537).read_to_string(&mut bytes)?;
    if bytes.len()>65536{return Err(io::Error::new(io::ErrorKind::InvalidData,"system summary exceeds limit"));}
    Ok(bytes)
}
pub fn system_snapshot()->SystemSnapshot{snapshot(Path::new("/proc"))}
fn snapshot(root:&Path)->SystemSnapshot{
    let (memory_total_kib,memory_used_kib)=read(root,"meminfo").ok().map(|s|meminfo(&s)).unwrap_or_default();
    let load_1m=read(root,"loadavg").ok().and_then(|s|s.split_whitespace().next()?.parse::<f64>().ok()).filter(|v|v.is_finite()&&*v>=0.0);
    SystemSnapshot{load_1m,memory_total_kib,memory_used_kib}
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn meminfo_uses_available_not_free(){
        assert_eq!(meminfo("MemTotal: 1000 kB\nMemFree: 50 kB\nMemAvailable: 400 kB\n"),(Some(1000),Some(600)));
        assert_eq!(meminfo("MemTotal: 1000 kB\nMemAvailable: 2000 kB\n"),(Some(1000),None));
        assert_eq!(meminfo("MemTotal: 1000 kB\n"),(Some(1000),None));
    }
}
