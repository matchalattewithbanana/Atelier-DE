//! Search and classification of installed desktop entries. No process scan,
//! command evaluation, network lookup or persisted usage history.
use gtk::{gio,prelude::*};
use std::collections::HashSet;

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Category{All,Pinned,Internet,Office,Development,Graphics,Media,Games,System,Utilities,Education,Other}
impl Category{
    pub const OPTIONS:&'static [(Self,&'static str)]=&[
        (Self::All,"All apps"),(Self::Pinned,"Pinned"),(Self::Internet,"Internet"),
        (Self::Office,"Office"),(Self::Development,"Development"),(Self::Graphics,"Graphics"),
        (Self::Media,"Media"),(Self::Games,"Games"),(Self::System,"System"),
        (Self::Utilities,"Utilities"),(Self::Education,"Education"),(Self::Other,"Other"),
    ];
    fn classify(value:&str)->Vec<Self>{
        let mut categories=Vec::new();
        for token in value.split(';'){
            let category=match token{
                "Network"=>Self::Internet,"Office"=>Self::Office,"Development"=>Self::Development,
                "Graphics"=>Self::Graphics,"AudioVideo"|"Audio"|"Video"=>Self::Media,
                "Game"=>Self::Games,"System"|"Settings"=>Self::System,"Utility"=>Self::Utilities,
                "Education"|"Science"=>Self::Education,_=>continue,
            };
            if !categories.contains(&category){categories.push(category);}
        }
        if categories.is_empty(){categories.push(Self::Other);}
        categories
    }
}
/// At most one insertion, deletion, substitution or adjacent swap.
fn near(a:&str,b:&str)->bool{
    let a:Vec<_>=a.chars().take(65).collect();let b:Vec<_>=b.chars().take(65).collect();
    if a.len()<4||a.len()>64||b.len()>64||a.len().abs_diff(b.len())>1{return false;}
    let i=a.iter().zip(&b).position(|(x,y)|x!=y).unwrap_or(a.len().min(b.len()));
    if a.len()==b.len(){
        if i==a.len(){return true;}
        return a[i+1..]==b[i+1..]||
            (i+1<a.len()&&a[i]==b[i+1]&&a[i+1]==b[i]&&a[i+2..]==b[i+2..]);
    }
    let (short,long)=if a.len()<b.len(){(&a,&b)}else{(&b,&a)};
    short[i..]==long[i+1..]
}
#[derive(Debug)]
struct SearchFields{name:String,id:String,metadata:String,name_words:Vec<String>}
impl SearchFields{
    fn new(name:&str,id:&str,metadata:&str)->Self{
        let name=name.to_lowercase();
        let name_words=name.split(|c:char|!c.is_alphanumeric()).filter(|w|!w.is_empty()).map(str::to_owned).collect();
        Self{name,id:id.to_lowercase(),metadata:metadata.to_lowercase(),name_words}
    }
    /// All query terms must match; title matches outrank identifier and
    /// metadata matches. The caller resolves equal ranks by catalog order.
    fn score(&self,query:&str)->Option<u32>{
        let query=query.trim().chars().take(512).collect::<String>().to_lowercase();
        if query.is_empty(){return Some(0);}
        let mut rank=if self.name==query{1000}else if self.name.starts_with(&query){500}else{0};
        let mut fuzzy=false;
        for word in query.split_whitespace(){
            rank+=if self.name==word{100}
                else if self.name_words.iter().any(|w|w.starts_with(word)){80}
                else if self.name.contains(word){60}
                else if self.id.contains(word){30}
                else if self.metadata.contains(word){10}
                else if self.name_words.iter().any(|w|near(word,w)){fuzzy=true;5}
                else{return None;};
        }
        // Every literal match outranks every typo-assisted match.
        Some(if fuzzy{rank}else{rank+100_000})
    }
}
pub struct Entry{
    pub app:gio::AppInfo,pub name:String,pub description:String,pub pinned:bool,
    categories:Vec<Category>,search:SearchFields,
}
impl Entry{
    pub fn rank(&self,category:Category,query:&str)->Option<u32>{
        if category!=Category::All&&!(category==Category::Pinned&&self.pinned)&&!self.categories.contains(&category){return None;}
        self.search.score(query)
    }
}
pub fn catalog(pins:&[String])->Vec<Entry>{
    let mut seen=HashSet::new();
    crate::applications::installed().into_iter().filter_map(|app|{
        let name=app.display_name().to_string();
        let id=app.id().map(|v|v.to_string());
        // Distinct entries without IDs need not share the same name.
        if let Some(id)=&id{if !seen.insert(id.clone()){return None;}}
        let desktop=app.clone().downcast::<gio::DesktopAppInfo>().ok();
        let categories=desktop.as_ref().and_then(|d|d.categories()).unwrap_or_default();
        let generic=desktop.as_ref().and_then(|d|d.generic_name()).unwrap_or_default();
        let keywords=desktop.as_ref().map(|d|d.keywords().iter().map(|s|s.as_str()).collect::<Vec<_>>().join(" ")).unwrap_or_default();
        let description=app.description().filter(|s|!s.trim().is_empty()).map(|s|s.to_string()).unwrap_or_else(||generic.to_string());
        let pinned=id.as_ref().is_some_and(|id|pins.contains(id));
        let metadata=format!("{description} {generic} {keywords} {categories}");
        let search=SearchFields::new(&name,id.as_deref().unwrap_or(""),&metadata);
        Some(Entry{app,name,description,pinned,categories:Category::classify(&categories),search})
    }).collect()
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn name_matches_rank_above_metadata_and_all_terms_required(){
        let browser=SearchFields::new("Firefox","org.mozilla.firefox.desktop","Web Browser Internet");
        let other=SearchFields::new("Documentation","docs.desktop","Firefox browser");
        assert!(browser.score("firefox").unwrap()>other.score("firefox").unwrap());
        assert!(browser.score("FIRE fox").is_some());
        assert!(browser.score("firefox internet").is_some());
        assert_eq!(browser.score("firefox missing"),None);
        assert_eq!(browser.score("   "),Some(0));
    }
    #[test]
    fn one_edit_name_fallback_is_bounded_and_lower_priority(){
        let browser=SearchFields::new("Firefox","org.mozilla.firefox.desktop","Web Browser");
        for typo in ["fireofx","firefx","firrefox","firefoc"]{assert!(browser.score(typo).is_some(),"{typo}");}
        assert!(browser.score("firxx").is_none());
        assert!(!near("fir","fox"));
        assert!(browser.score("fireofx browser").is_some());
        assert!(browser.score("fireofx missing").is_none());
        let literal=SearchFields::new("Other","other.desktop","fireofx");
        assert!(literal.score("fireofx").unwrap()>browser.score("fireofx").unwrap());
        assert!(near("café","cafe"));
    }
    #[test]
    fn category_membership_is_exact_and_multi_valued(){
        assert_eq!(Category::classify("AudioVideo;Audio;Network;"),vec![Category::Media,Category::Internet]);
        assert_eq!(Category::classify("Unknown;NetworkExtra;"),vec![Category::Other]);
    }
}
