//! Built-in visual presets for the native portable-theme format. This catalog
//! has no machine-local settings, commands, wallpaper paths or rice downloads.
use super::{from_seed_with,PortableTheme};
use crate::{Appearance,Transparency,WindowSpacing,WindowShadow};
use crate::dynamic_palette::{Options,Variant,Contrast};
use anyhow::{Context,Result};

#[derive(Clone,Copy)]
pub struct Preset{
    pub id:&'static str,
    pub name:&'static str,
    pub detail:&'static str,
    seed:[u8;3],
    light:bool,
    variant:Variant,
    contrast:Contrast,
    radius:u32,
    panel:u32,
    spacing:u32,
    icons:u32,
    gaps:(u32,u32),
    border:u32,
    corners:u32,
    shadow:u32,
    glass:bool,
}
pub const CATALOG:&[Preset]=&[
    Preset{id:"midnight",name:"Midnight",detail:"Dark · calm blue · rounded solid surfaces",seed:[53,77,122],light:false,variant:Variant::TonalSpot,contrast:Contrast::Normal,radius:14,panel:40,spacing:8,icons:36,gaps:(8,12),border:1,corners:12,shadow:16,glass:false},
    Preset{id:"blue-glass",name:"Blue Glass",detail:"Dark · translucent blue · experimental backdrop blur",seed:[40,91,180],light:false,variant:Variant::TonalSpot,contrast:Contrast::Normal,radius:18,panel:44,spacing:10,icons:40,gaps:(10,16),border:1,corners:16,shadow:20,glass:true},
    Preset{id:"daylight",name:"Daylight",detail:"Light · soft neutral · spacious solid surfaces",seed:[132,151,170],light:true,variant:Variant::TonalSpot,contrast:Contrast::Normal,radius:14,panel:44,spacing:10,icons:36,gaps:(8,12),border:1,corners:12,shadow:12,glass:false},
    Preset{id:"sakura",name:"Sakura",detail:"Light · rose-inspired expressive palette · soft curves",seed:[209,105,150],light:true,variant:Variant::Expressive,contrast:Contrast::Normal,radius:24,panel:48,spacing:12,icons:40,gaps:(12,18),border:2,corners:20,shadow:18,glass:false},
    Preset{id:"forest",name:"Forest",detail:"Dark · green tonal palette · natural restrained geometry",seed:[48,125,86],light:false,variant:Variant::TonalSpot,contrast:Contrast::Normal,radius:12,panel:40,spacing:8,icons:32,gaps:(8,12),border:2,corners:10,shadow:12,glass:false},
    Preset{id:"monochrome",name:"Monochrome Contrast",detail:"Dark · high contrast monochrome · compact opaque surfaces",seed:[128,128,128],light:false,variant:Variant::Monochrome,contrast:Contrast::High,radius:4,panel:36,spacing:4,icons:32,gaps:(4,6),border:2,corners:4,shadow:0,glass:false},
];

impl Preset{
    /// A full portable visual draft, independent of any previous draft.
    /// Local settings remain untouched when applied through PortableTheme.
    pub fn theme(&self)->Result<PortableTheme>{
        let mut t=from_seed_with(self.seed,Options{variant:self.variant,light:self.light,contrast:self.contrast},&Appearance::default())?;
        t.name=self.name.into();
        t.button_radius=self.radius;t.panel_height=self.panel;t.panel_spacing=self.spacing;
        t.launcher_icon_size=self.icons;t.dock_icon_size=self.icons;
        t.window_spacing=WindowSpacing{inner_gap:self.gaps.0,outer_gap:self.gaps.1};
        t.window_border.width=self.border;t.window_corner_radius=self.corners;
        t.window_shadow=WindowShadow{radius:self.shadow,opacity:32};
        if self.glass{
            t.panel_backdrop_blur=8;t.dock_backdrop_blur=8;t.widget_backdrop_blur=8;
            t.transparency=Transparency{focused:100,unfocused:94,panel:82,dock:82,widgets:88};
        }
        t.applied(&Appearance::default())?;Ok(t)
    }
    pub fn matches(&self,query:&str)->bool{
        let haystack=format!("{} {} {}",self.id,self.name,self.detail).to_lowercase();
        query.split_whitespace().all(|word|haystack.contains(&word.to_lowercase()))
    }
}
pub fn by_id(id:&str)->Result<PortableTheme>{
    CATALOG.iter().find(|p|p.id==id).context("unknown native theme preset")?.theme()
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn all_presets_validate_and_roundtrip(){
        let mut ids=std::collections::HashSet::new();
        for p in CATALOG{
            assert!(ids.insert(p.id));
            let t=p.theme().unwrap();assert_eq!(t,by_id(p.id).unwrap());
            assert_eq!(PortableTheme::parse(&t.json().unwrap()).unwrap(),t);
        }
        assert!(by_id("unknown").is_err());
    }
    #[test]
    fn presets_never_overwrite_machine_local_preferences(){
        let mut base=Appearance::default();
        base.wallpaper_path="/tmp/example.png".into();
        base.dock_pins=vec!["foot.desktop".into()];base.dock_enabled=true;
        base.motion.reduced_motion=true;base.widgets.enabled=true;
        base.notifications.timeout_seconds=10;
        for p in CATALOG{
            let a=p.theme().unwrap().applied(&base).unwrap();
            assert_eq!(a.wallpaper_path,base.wallpaper_path);assert_eq!(a.dock_pins,base.dock_pins);
            assert_eq!(a.dock_enabled,base.dock_enabled);assert_eq!(a.motion,base.motion);
            assert_eq!(a.widgets,base.widgets);assert_eq!(a.notifications,base.notifications);
        }
    }
    #[test]
    fn search_and_effect_profiles_are_distinct(){
        assert!(CATALOG.iter().find(|p|p.id=="blue-glass").unwrap().matches("BLUE blur"));
        assert!(CATALOG.iter().find(|p|p.id=="monochrome").unwrap().matches("high contrast"));
        assert!(!CATALOG[0].matches("nonexistent"));
        let glass=by_id("blue-glass").unwrap();let solid=by_id("midnight").unwrap();
        assert!(glass.panel_backdrop_blur>0&&glass.transparency.panel<100);
        assert_eq!(solid.panel_backdrop_blur,0);assert_eq!(solid.transparency.panel,100);
        let mono=by_id("monochrome").unwrap();let p=mono.shell_palette.unwrap();
        for bg in [&p.surface,&p.elevated]{for fg in [&p.text,&p.muted]{
            assert!(super::super::contrast(super::super::rgb(bg).unwrap(),super::super::rgb(fg).unwrap())>=7.0);
        }}
    }
}
