//! Read-only static samples from installed XCursor files.
use anyhow::{bail,Result};
use atelier_config::{CursorPreferences,cursor_images::{self,CursorFrame}};
const SHAPES:&[(&str,&[&str])]=&[
    ("Default",&["default","left_ptr"]),("Link",&["pointer","hand2","hand1"]),
    ("Text",&["text","xterm"]),("Wait",&["wait","watch"]),
    ("Resize",&["ew-resize","sb_h_double_arrow"]),("Move",&["move","fleur"]),
];
pub struct Sample{pub label:&'static str,pub frame:Option<CursorFrame>}
pub fn samples(theme:&str,size:u32)->Result<Vec<Sample>>{
    CursorPreferences{theme:theme.into(),size}.validate()?;
    if theme.is_empty(){bail!("enter an installed theme name");}
    let roots=crate::customization::icon_roots();
    let mut result=Vec::new();
    for (label,names) in SHAPES{
        let frame=roots.iter().find_map(|root|names.iter().find_map(|name|{
            let path=root.join(theme).join("cursors").join(name);
            cursor_images::load(&path,size).ok()?.into_iter().next()
        }));
        result.push(Sample{label,frame});
    }
    if result.iter().all(|s|s.frame.is_none()){bail!("no valid direct cursor images found for this installed theme");}
    Ok(result)
}
