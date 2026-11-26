//! Private XKB decoder and password-input gate. The protocol adapter must
//! identify the exact lock surface and deliver compositor modifier masks.
//! No input is accepted until the visible prompt, lock and focus are ready.
use crate::entry::Entry;
use anyhow::{bail,Context,Result};
use std::{collections::BTreeSet,fs::File,os::{fd::OwnedFd,unix::fs::FileExt},time::Instant};
use xkbcommon::xkb;

const MAX_KEYMAP:usize=1024*1024;
const MAX_HELD:usize=256;

fn keymap_text(fd:OwnedFd,size:u32)->Result<String>{
    let file=File::from(fd);
    let size=size as usize;
    if !(2..=MAX_KEYMAP).contains(&size){bail!("invalid keymap size");}
    let metadata=file.metadata()?;
    if !metadata.is_file()||metadata.len()<size as u64{bail!("invalid keymap backing file");}
    // Copy the advertised range. Do not mmap mutable server storage or read
    // beyond the declared range; positional reads avoid shared offsets.
    let mut bytes=vec![0;size];file.read_exact_at(&mut bytes,0)?;
    if bytes.pop()!=Some(0)||bytes.contains(&0){bail!("invalid keymap terminator");}
    String::from_utf8(bytes).context("keymap is not UTF-8")
}

struct Map{xkb:xkb::State}
impl Map{
    fn new(text:String)->Result<Self>{
        // A serialized keymap should be self-contained. Do not resolve local
        // user/environment include paths while parsing compositor input.
        let context=xkb::Context::new(xkb::CONTEXT_NO_DEFAULT_INCLUDES|xkb::CONTEXT_NO_ENVIRONMENT_NAMES);
        let map=xkb::Keymap::new_from_string(&context,text,xkb::KEYMAP_FORMAT_TEXT_V1,xkb::KEYMAP_COMPILE_NO_FLAGS)
            .context("invalid XKB keymap")?;
        Ok(Self{xkb:xkb::State::new(&map)})
    }
    fn modifiers(&mut self,depressed:u32,latched:u32,locked:u32,group:u32){
        self.xkb.update_mask(depressed,latched,locked,0,0,group);
    }
    fn edit(&self,evdev:u32)->Option<Edit>{
        let key=xkb::Keycode::new(evdev.checked_add(8)?);
        // Reject command shortcuts rather than interpreting Ctrl+letter as
        // password text. Shift, Caps Lock and layout selection remain XKB's.
        if ["Control","Mod1","Mod4"].iter().any(|name|self.xkb.mod_name_is_active(*name,xkb::STATE_MODS_EFFECTIVE)){
            return None;
        }
        match self.xkb.key_get_one_sym(key).raw(){
            xkb::keysyms::KEY_Return|xkb::keysyms::KEY_KP_Enter=>Some(Edit::Submit),
            xkb::keysyms::KEY_BackSpace=>Some(Edit::Backspace),
            xkb::keysyms::KEY_Escape=>Some(Edit::Cancel),
            _=>char::from_u32(self.xkb.key_get_utf32(key)).filter(|ch|!ch.is_control()).map(Edit::Insert),
        }
    }
}
// Deliberately not Debug or Clone: a character may contain password material.
enum Edit{Insert(char),Backspace,Cancel,Submit}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(super) enum Intent{None,Changed,Submit}
#[derive(Default)]
pub(super) struct Keyboard{
    map:Option<Map>,held:BTreeSet<u32>,focused:bool,visible:bool,modifiers_ready:bool,
}
impl Keyboard{
    fn sync(&self,entry:&mut Entry){
        entry.set_ready(self.focused&&self.visible&&self.modifiers_ready&&self.map.is_some());
    }
    pub(super) fn replace_map(&mut self,entry:&mut Entry,fd:OwnedFd,size:u32)->Result<()>{
        self.map=None;self.modifiers_ready=false;entry.set_ready(false);
        self.map=Some(Map::new(keymap_text(fd,size)?)?);
        Ok(())
    }
    pub(super) fn prompt_ready(&mut self,entry:&mut Entry,ready:bool){
        self.visible=ready;self.sync(entry);
    }
    pub(super) fn enter(&mut self,entry:&mut Entry,exact_surface:bool,keys:&[u8])->Result<()>{
        self.leave(entry);
        if !exact_surface{return Ok(());}
        if keys.len()%4!=0||keys.len()/4>MAX_HELD{bail!("invalid keyboard enter keys");}
        for bytes in keys.chunks_exact(4){
            let key=u32::from_ne_bytes(bytes.try_into().unwrap());
            if key.checked_add(8).is_none()||!self.held.insert(key){self.held.clear();bail!("invalid held key");}
        }
        self.focused=true;self.sync(entry);Ok(())
    }
    pub(super) fn leave(&mut self,entry:&mut Entry){
        self.focused=false;self.modifiers_ready=false;self.held.clear();entry.set_ready(false);
    }
    pub(super) fn remove(&mut self,entry:&mut Entry){
        self.leave(entry);self.map=None;
    }
    pub(super) fn modifiers(&mut self,entry:&mut Entry,depressed:u32,latched:u32,locked:u32,group:u32){
        if let Some(map)=&mut self.map{
            map.modifiers(depressed,latched,locked,group);
            self.modifiers_ready=self.focused;self.sync(entry);
        }
    }
    pub(super) fn key(&mut self,entry:&mut Entry,key:u32,pressed:bool,now:Instant)->Intent{
        if !self.focused||key.checked_add(8).is_none(){return Intent::None;}
        if !pressed{self.held.remove(&key);return Intent::None;}
        if self.held.contains(&key){return Intent::None;}
        if self.held.len()>=MAX_HELD{self.leave(entry);return Intent::None;}
        self.held.insert(key);
        if !self.visible||!self.modifiers_ready{return Intent::None;}
        let Some(edit)=self.map.as_ref().and_then(|map|map.edit(key))else{return Intent::None;};
        match edit{
            Edit::Insert(ch) if entry.insert(ch,now)=>Intent::Changed,
            Edit::Backspace if entry.backspace(now)=>Intent::Changed,
            Edit::Cancel=>{entry.cancel();Intent::Changed}
            Edit::Submit=>Intent::Submit,
            _=>Intent::None,
        }
    }
}

#[cfg(test)]
mod tests{
    use super::*;
    use std::io::Write;
    // Generate a self-contained map using the installed system XKB data,
    // then exercise the same decoder used for compositor-provided maps.
    fn fixture()->String{
        let context=xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        xkb::Keymap::new_from_names(&context,"evdev","pc105","us","",None,xkb::KEYMAP_COMPILE_NO_FLAGS)
            .unwrap().get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1)
    }
    fn file(bytes:&[u8])->OwnedFd{
        let mut file=tempfile::tempfile().unwrap();file.write_all(bytes).unwrap();file.into()
    }
    fn install(keyboard:&mut Keyboard,entry:&mut Entry){
        let mut bytes=fixture().into_bytes();bytes.push(0);
        keyboard.replace_map(entry,file(&bytes),bytes.len() as u32).unwrap();
    }
    #[test]
    fn bounded_keymap_file_and_terminator(){
        assert_eq!(keymap_text(file(b"abc\0"),4).unwrap(),"abc");
        for bytes in [b"abc".as_slice(),b"a\0b\0",b"\xff\0"]{
            assert!(keymap_text(file(bytes),bytes.len() as u32).is_err());
        }
        assert!(keymap_text(file(b"x\0"),0).is_err());
        assert!(keymap_text(file(b"x\0"),3).is_err());
        assert!(keymap_text(file(b"x\0"),MAX_KEYMAP as u32+1).is_err());
    }
    #[test]
    fn focus_prompt_and_modifier_gates_and_held_keys(){
        let now=Instant::now();let mut keyboard=Keyboard::default();let mut entry=Entry::default();
        install(&mut keyboard,&mut entry);
        keyboard.enter(&mut entry,true,&30u32.to_ne_bytes()).unwrap();
        keyboard.prompt_ready(&mut entry,true);
        assert_eq!(keyboard.key(&mut entry,31,true,now),Intent::None);
        keyboard.modifiers(&mut entry,0,0,0,0);
        assert_eq!(keyboard.key(&mut entry,30,true,now),Intent::None);
        keyboard.key(&mut entry,30,false,now);
        assert_eq!(keyboard.key(&mut entry,30,true,now),Intent::Changed);
        assert!(entry.has_input());
        assert_eq!(keyboard.key(&mut entry,30,true,now),Intent::None);
        assert_eq!(keyboard.key(&mut entry,28,true,now),Intent::Submit);
        keyboard.leave(&mut entry);assert!(!entry.has_input());
        keyboard.enter(&mut entry,false,&[]).unwrap();
        assert_eq!(keyboard.key(&mut entry,30,true,now),Intent::None);
    }
    #[test]
    fn replacement_invalidates_input_and_invalid_enter_never_enables(){
        let now=Instant::now();let mut keyboard=Keyboard::default();let mut entry=Entry::default();
        install(&mut keyboard,&mut entry);keyboard.enter(&mut entry,true,&[]).unwrap();
        keyboard.prompt_ready(&mut entry,true);keyboard.modifiers(&mut entry,0,0,0,0);
        assert_eq!(keyboard.key(&mut entry,30,true,now),Intent::Changed);
        assert!(keyboard.replace_map(&mut entry,file(b"bad\0"),4).is_err());
        assert!(!entry.has_input());
        assert!(keyboard.enter(&mut entry,true,&[1,2,3]).is_err());
        assert_eq!(keyboard.key(&mut entry,28,true,now),Intent::None);
        keyboard.remove(&mut entry);
    }
    #[test]
    fn xkb_shift_and_shortcuts_use_server_masks(){
        let mut map=Map::new(fixture()).unwrap();
        assert!(matches!(map.edit(30),Some(Edit::Insert('a'))));
        let keymap=map.xkb.get_keymap();
        let shift=1u32<<keymap.mod_get_index("Shift");
        let control=1u32<<keymap.mod_get_index("Control");
        map.modifiers(shift,0,0,0);assert!(matches!(map.edit(30),Some(Edit::Insert('A'))));
        map.modifiers(control,0,0,0);assert!(map.edit(30).is_none());assert!(map.edit(28).is_none());
        map.modifiers(0,0,0,0);assert!(matches!(map.edit(14),Some(Edit::Backspace)));
        assert!(matches!(map.edit(1),Some(Edit::Cancel)));assert!(map.edit(u32::MAX).is_none());
    }
}
