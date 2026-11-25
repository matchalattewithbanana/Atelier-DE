//! Read-only public community discovery. Repository metadata is not trust.
use anyhow::{bail,Context,Result};
use serde::{Deserialize,Serialize};
use std::collections::BTreeSet;

const PAGE_LIMIT:usize=20;
const TOPICS:&[(&str,&str)]=&[
    ("cursor","xcursor"),("login","sddm-theme"),("locker","hyprlock"),
    ("icons","icon-theme"),("application-theme","gtk-theme"),("wallpaper","wallpapers"),
    ("shell","quickshell"),("rice","linux-rice"),
];

#[derive(Deserialize)]
struct ApiPage{
    total_count:u64,
    incomplete_results:bool,
    items:Vec<ApiRepo>,
}
#[derive(Deserialize)]
struct ApiRepo{
    id:u64,full_name:String,description:Option<String>,
    private:bool,archived:bool,disabled:bool,fork:bool,
    stargazers_count:u64,license:Option<ApiLicense>,updated_at:Option<String>,
}
#[derive(Deserialize)]
struct ApiLicense{spdx_id:Option<String>}

#[derive(Debug,Serialize)]
pub struct Listing{
    pub repository:String,
    pub source:String,
    pub description:String,
    pub stars:u64,
    pub license:String,
    pub updated_at:String,
    pub compatibility:&'static str,
    pub trust:&'static str,
}
#[derive(Debug,Serialize)]
pub struct Search{
    pub schema:u32,
    pub category:String,
    pub query:String,
    pub total_matches:u64,
    pub incomplete:bool,
    pub listings:Vec<Listing>,
    pub note:&'static str,
}
fn query(category:&str,keywords:&str)->Result<String>{
    let topic=TOPICS.iter().find(|(id,_)|*id==category).map(|(_,topic)|*topic).context("select a supported discovery category")?;
    let keywords=keywords.trim();
    if keywords.len()>80||!keywords.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b' '|b'-'|b'_'|b'.')){
        bail!("keywords must be up to 80 ASCII letters, digits, spaces, dots, underscores or hyphens");
    }
    // Prevent user text from negating filters or supplying search operators.
    if keywords.split_whitespace().any(|word|!word.as_bytes()[0].is_ascii_alphanumeric()){
        bail!("each search word must begin with a letter or digit");
    }
    Ok(format!("topic:{topic} is:public archived:false fork:false {keywords}").trim_end().to_owned())
}
fn valid_repo(name:&str)->bool{
    let Some((owner,repo))=name.split_once('/')else{return false;};
    !owner.is_empty()&&owner.len()<=39&&owner.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-')
        &&!repo.is_empty()&&repo.len()<=100&&repo!="."&&repo!=".."
        &&repo.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'-'|b'_'|b'.'))
}
fn plain(text:Option<String>,limit:usize)->String{
    // Remove control characters and bidirectional formatting rather than
    // displaying terminal escapes or misleading directional overrides.
    text.unwrap_or_default().chars().filter(|c|!c.is_control()&&!matches!(*c,
        '\u{061c}'|'\u{200e}'|'\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))
        .take(limit).collect()
}
fn parse(bytes:&[u8],category:&str,query:String)->Result<Search>{
    let page:ApiPage=serde_json::from_slice(bytes).context("invalid GitHub repository search response")?;
    if page.items.len()>PAGE_LIMIT{bail!("unexpected search page size");}
    let mut ids=BTreeSet::new();let mut names=BTreeSet::new();let mut listings=Vec::new();
    for item in page.items{
        if item.id==0||item.private||item.archived||item.disabled||item.fork||!valid_repo(&item.full_name)
            ||!ids.insert(item.id)||!names.insert(item.full_name.to_ascii_lowercase()){continue;}
        let source=format!("https://github.com/{}",item.full_name);
        let license=plain(item.license.and_then(|l|l.spdx_id),64);
        listings.push(Listing{repository:item.full_name,source,description:plain(item.description,500),
            stars:item.stargazers_count,license:if license.is_empty()||license=="NOASSERTION"{"Unknown".into()}else{license},
            updated_at:plain(item.updated_at,40),compatibility:"Not verified",trust:"Community reference; not trusted or installable"});
    }
    Ok(Search{schema:1,category:category.into(),query,total_matches:page.total_count,incomplete:page.incomplete_results,
        listings,note:"GitHub topic search, first page only. Stars and license metadata are advisory. No code, assets or manifests downloaded; no registration or installation."})
}
pub fn discover(category:&str,keywords:&str)->Result<Search>{
    let query=query(category,keywords)?;
    let bytes=crate::discovery::search_repositories(&query)?;
    parse(&bytes,category,query)
}
pub fn run(args:&[String])->Result<()>{
    let json=args.last().is_some_and(|a|a=="--json");
    let args=if json{&args[..args.len()-1]}else{args};
    let (category,keywords)=match args{
        [category]=>(category.as_str(),""),
        [category,keywords]=>(category.as_str(),keywords.as_str()),
        _=>bail!("use customizations discover CATEGORY [KEYWORDS] [--json]"),
    };
    let result=discover(category,keywords)?;
    if json{println!("{}",serde_json::to_string_pretty(&result)?);}
    else{
        println!("Community discovery — {} · {} reported matches · up to {PAGE_LIMIT} results",category,result.total_matches);
        if result.incomplete{println!("GitHub marked the response incomplete.");}
        for item in &result.listings{println!("{}\n  {}\n  {} stars · license: {} · updated: {}\n  {}\n  {}",item.repository,item.source,item.stars,item.license,item.updated_at,item.description,item.trust);}
        println!("{}",result.note);
    }
    Ok(())
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn search_is_category_bounded(){
        assert_eq!(query("login","dark blue").unwrap(),"topic:sddm-theme is:public archived:false fork:false dark blue");
        for value in ["repo:evil/test","--hostname","-archived","x OR is:private","x&per_page=100","x\nz"]{assert!(query("cursor",value).is_err());}
        assert!(query("unknown","").is_err());
    }
    fn repo(id:u64,name:&str)->serde_json::Value{
        serde_json::json!({"id":id,"full_name":name,"description":"nice\u{001b}\u{202e} theme",
            "private":false,"archived":false,"disabled":false,"fork":false,"stargazers_count":10,
            "license":{"spdx_id":"GPL-3.0"},"updated_at":"2026-01-01T00:00:00Z",
            "html_url":"https://untrusted.example/redirect","install_command":"ignored"})
    }
    #[test]
    fn response_is_reference_only_and_deduplicated(){
        let good=repo(1,"example/theme");let mut private=repo(2,"example/private");private["private"]=true.into();
        let mut archived=repo(3,"example/archive");archived["archived"]=true.into();
        let data=serde_json::to_vec(&serde_json::json!({"total_count":8,"incomplete_results":true,
            "items":[good.clone(),good,private,archived,repo(4,"bad/../name"),repo(5,"Example/Theme")]})).unwrap();
        let result=parse(&data,"cursor","query".into()).unwrap();
        assert!(result.incomplete);assert_eq!(result.listings.len(),1);
        let entry=&result.listings[0];assert_eq!(entry.source,"https://github.com/example/theme");
        assert_eq!(entry.description,"nice theme");assert_eq!(entry.compatibility,"Not verified");
        assert!(entry.trust.contains("not trusted"));
    }
    #[test]
    fn rejects_excessive_response_pages(){
        let data=serde_json::to_vec(&serde_json::json!({"total_count":21,"incomplete_results":false,
            "items":vec![repo(1,"example/theme");21]})).unwrap();
        assert!(parse(&data,"cursor","query".into()).is_err());
    }
}
