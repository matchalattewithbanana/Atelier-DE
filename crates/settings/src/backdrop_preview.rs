//! Bounded synthetic backdrop comparison, independent of compositor rendering.
use atelier_config::themes::PortableTheme;
use gtk::{cairo::{Context,Format,ImageSurface},gdk};

const W:usize=160;
const H:usize=48;
type Pixel=[u8;3];

fn effective_radius(radius:u32,opacity:u32)->usize{
    if opacity>=100{0}else{radius.min(16) as usize}
}

fn pattern()->Vec<Pixel>{
    (0..W*H).map(|i|{
        let (x,y)=(i%W,i/W);
        match (x/12+y/12)%3{0=>[48,79,128],1=>[180,151,210],_=>[224,185,112]}
    }).collect()
}

/// Two separable box passes over a fixed-size sample, with clamped edges.
/// Radius zero is an exact identity; the real GPU filter may differ.
fn blur(input:&[Pixel],radius:usize)->Vec<Pixel>{
    let radius=radius.min(16);
    if radius==0{return input.to_vec();}
    let mut tmp=vec![[0;3];W*H];
    let mut out=tmp.clone();
    let count=(radius*2+1) as u32;
    for y in 0..H{for x in 0..W{
        let mut sum=[0u32;3];
        for offset in 0..=radius*2{
            let sx=(x+offset).saturating_sub(radius).min(W-1);
            for c in 0..3{sum[c]+=u32::from(input[y*W+sx][c]);}
        }
        tmp[y*W+x]=sum.map(|n|(n/count) as u8);
    }}
    for y in 0..H{for x in 0..W{
        let mut sum=[0u32;3];
        for offset in 0..=radius*2{
            let sy=(y+offset).saturating_sub(radius).min(H-1);
            for c in 0..3{sum[c]+=u32::from(tmp[sy*W+x][c]);}
        }
        out[y*W+x]=sum.map(|n|(n/count) as u8);
    }}
    out
}

fn surface(pixels:&[Pixel])->Result<ImageSurface,gtk::cairo::Error>{
    // Cairo RGB24 is native-endian 0x00RRGGBB, not byte-order RGBA.
    let bytes:Vec<u8>=pixels.iter().flat_map(|&[r,g,b]|(
        (u32::from(r)<<16)|(u32::from(g)<<8)|u32::from(b)
    ).to_ne_bytes()).collect();
    ImageSurface::create_for_data(bytes,Format::Rgb24,W as i32,H as i32,(W*4) as i32)
}
fn rgb(color:&str,fallback:[f64;3])->[f64;3]{
    gdk::RGBA::parse(color).map(|c|[c.red() as f64,c.green() as f64,c.blue() as f64]).unwrap_or(fallback)
}
fn paint_sample(cr:&Context,x:f64,y:f64,img:&ImageSurface){
    let _=cr.save();
    cr.translate(x,y);cr.rectangle(0.0,0.0,W as f64,H as f64);cr.clip();
    if cr.set_source_surface(img,0.0,0.0).is_ok(){let _=cr.paint();}
    let _=cr.restore();
}
fn label(cr:&Context,x:f64,y:f64,size:f64,text:&str,color:[f64;3]){
    cr.set_source_rgb(color[0],color[1],color[2]);cr.set_font_size(size);cr.move_to(x,y);let _=cr.show_text(text);
}
pub(super) fn draw(cr:&Context,width:i32,height:i32,t:&PortableTheme){
    if width<=0||height<=0{return;}
    let scale=(f64::from(width)/440.0).min(f64::from(height)/230.0);
    let _=cr.save();
    cr.translate((f64::from(width)-440.0*scale)/2.0,(f64::from(height)-230.0*scale)/2.0);cr.scale(scale,scale);
    let (base,fg)=t.shell_palette.as_ref().map(|p|(rgb(&p.elevated,[0.19,0.22,0.28]),rgb(&p.text,[0.95,0.95,0.95]))).unwrap_or(([0.19,0.22,0.28],[0.95,0.95,0.95]));
    cr.set_source_rgb(0.12,0.15,0.20);cr.rectangle(0.0,0.0,440.0,230.0);let _=cr.fill();
    let original=pattern();
    let Ok(sharp)=surface(&original)else{let _=cr.restore();return;};
    let samples=[
        ("Focused window",t.backdrop_blur,t.transparency.focused,true),
        ("Panel",t.panel_backdrop_blur,t.transparency.panel,false),
        ("Dock",t.dock_backdrop_blur,t.transparency.dock,false),
        ("Widget card",t.widget_backdrop_blur,t.transparency.widgets,false),
    ];
    for (i,(name,radius,opacity,client)) in samples.into_iter().enumerate(){
        let x=12.0+(i%2) as f64*214.0;let y=12.0+(i/2) as f64*108.0;
        label(cr,x,y+12.0,13.0,name,[0.94,0.96,1.0]);
        let effective=effective_radius(radius,opacity);
        let detail=if opacity>=100{"Opaque · blur inactive".to_owned()}else{format!("{radius} px blur · {opacity}% opacity")};
        label(cr,x,y+27.0,10.0,&detail,[0.72,0.78,0.88]);
        paint_sample(cr,x,y+34.0,&sharp);
        // Compare the same sharp pattern on the left with its filtered,
        // tinted counterpart on the right. Keep shell foreground opaque.
        let _=cr.save();cr.rectangle(x+80.0,y+34.0,80.0,48.0);cr.clip();
        if effective>0{if let Ok(img)=surface(&blur(&original,effective)){paint_sample(cr,x,y+34.0,&img);}}
        let a=f64::from(opacity.min(100))/100.0;
        if client{cr.push_group();}
        cr.set_source_rgba(base[0],base[1],base[2],if client{1.0}else{a});let _=cr.paint();
        cr.set_source_rgb(fg[0],fg[1],fg[2]);
        cr.set_font_size(17.0);cr.move_to(x+103.0,y+64.0);let _=cr.show_text("Aa");
        if client{let _=cr.pop_group_to_source();let _=cr.paint_with_alpha(a);}
        let _=cr.restore();
        label(cr,x,y+95.0,10.0,"Reference    /    Effect",[0.72,0.78,0.88]);
    }
    let _=cr.restore();
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn opacity_and_radius_gate(){
        assert_eq!(effective_radius(16,100),0);
        assert_eq!(effective_radius(0,70),0);
        assert_eq!(effective_radius(8,82),8);
        assert_eq!(effective_radius(99,82),16);
    }
    #[test]
    fn blur_preserves_constants_and_softens_edges(){
        let constant=vec![[17,80,121];W*H];
        assert_eq!(blur(&constant,16),constant);
        let edge:Vec<_>=(0..W*H).map(|i|if i%W<W/2{[0;3]}else{[255;3]}).collect();
        assert_eq!(blur(&edge,0),edge);
        let softened=blur(&edge,4);
        let sample=softened[W/2-1][0];
        assert!(sample>0&&sample<255);
        assert_eq!(softened[0],[0;3]);
    }
    #[test]
    fn bounded_preview_renders_all_effects(){
        let image=ImageSurface::create(Format::ARgb32,440,230).unwrap();
        let cr=Context::new(&image).unwrap();
        let mut a=atelier_config::Appearance::default();
        a.backdrop_blur=16;a.panel_backdrop_blur=8;a.dock_backdrop_blur=4;a.widget_backdrop_blur=12;
        a.transparency=atelier_config::Transparency{focused:70,unfocused:100,panel:82,dock:100,widgets:65};
        let theme=PortableTheme::capture("Sample".into(),&a);
        draw(&cr,440,230,&theme);
        assert!(cr.status().is_ok());
    }
}
