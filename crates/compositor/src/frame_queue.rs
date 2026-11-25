//! Two-slot scanout scheduling and conservative damage history, independent of DRM.
use smithay::utils::{Physical,Rectangle};

#[derive(Clone,Copy,Debug,Default,PartialEq,Eq,PartialOrd,Ord)]
pub enum InputDamage {
    #[default]
    None,
    Cursor,
    Scene,
}
#[derive(Debug)]
pub struct FrameQueue {
    dirty: bool,
    displayed: Option<usize>,
    pending: Option<usize>,
    // Each retained GBM slot has its own scene validity and old cursor.
    stale: [bool; 2],
    cursors: [Option<Rectangle<i32,Physical>>; 2],
}

impl Default for FrameQueue {
    fn default() -> Self { Self { dirty: true, displayed: None, pending: None, stale: [true;2], cursors: [None;2] } }
}

impl FrameQueue {
    pub fn invalidate(&mut self) {
        self.dirty = true;
        self.forget_damage_history();
    }
    pub fn invalidate_cursor(&mut self) { self.dirty = true; }

    /// Force a full redraw on the next submission of either slot, without
    /// waking an idle desktop merely to refresh a damage baseline.
    pub fn forget_damage_history(&mut self) { self.stale = [true;2]; }

    pub fn damage(&self, slot:usize, output:Rectangle<i32,Physical>, cursor:Rectangle<i32,Physical>) -> Rectangle<i32,Physical> {
        if self.stale[slot] { return output; }
        let Some(old) = self.cursors[slot] else { return output; };
        // Repaint both the cursor that was actually drawn into THIS buffer
        // and the new cursor. A bounding box avoids overlapping draw damage.
        let l=old.loc.x.min(cursor.loc.x);
        let t=old.loc.y.min(cursor.loc.y);
        let r=(old.loc.x+old.size.w).max(cursor.loc.x+cursor.size.w);
        let b=(old.loc.y+old.size.h).max(cursor.loc.y+cursor.size.h);
        Rectangle::new((l,t).into(),(r-l,b-t).into()).intersection(output).unwrap_or(output)
    }
    pub fn displayed(&self) -> Option<usize> { self.displayed }
    pub fn pending(&self) -> Option<usize> { self.pending }
    pub fn next(&self) -> Option<usize> {
        if !self.dirty || self.pending.is_some() { return None; }
        Some(match self.displayed { Some(0) => 1, _ => 0 })
    }
    /// Call only after a successful kernel submission of the slot from next().
    pub fn submitted(&mut self, slot: usize, cursor:Rectangle<i32,Physical>) {
        self.stale[slot]=false;
        self.cursors[slot]=Some(cursor);
        self.pending = Some(slot);
        self.dirty = false;
    }
    /// A matching page-flip completion is the only event that releases the former slot.
    pub fn completed(&mut self) -> Option<usize> {
        let slot = self.pending.take()?;
        self.displayed = Some(slot);
        Some(slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cursor(x:i32,y:i32)->Rectangle<i32,Physical>{Rectangle::new((x,y).into(),(11,16).into())}
    #[test]
    fn never_reuses_displayed_or_pending_slot() {
        let mut queue = FrameQueue::default();
        assert_eq!(queue.next(), Some(0));
        queue.submitted(0, cursor(0,0));
        queue.invalidate();
        assert_eq!(queue.next(), None);
        assert_eq!(queue.completed(), Some(0));
        assert_eq!(queue.next(), Some(1));
        queue.submitted(1, cursor(0,0));
        assert_eq!(queue.next(), None);
        assert_eq!(queue.completed(), Some(1));
        assert_eq!(queue.next(), None);
        queue.invalidate();
        assert_eq!(queue.next(), Some(0));
    }

    #[test]
    fn cursor_damage_tracks_each_buffer_and_scene_invalidation(){
        let mut queue=FrameQueue::default();
        let output=Rectangle::from_size((1920,1080).into());
        let a=cursor(10,10);let b=cursor(50,50);let c=cursor(30,20);
        assert_eq!(queue.damage(0,output,a),output);
        queue.submitted(0,a);queue.completed();
        queue.invalidate_cursor();
        assert_eq!(queue.damage(1,output,b),output);
        queue.submitted(1,b);queue.completed();
        queue.invalidate_cursor();
        assert_eq!(queue.damage(0,output,c),Rectangle::new((10,10).into(),(31,26).into()));
        queue.submitted(0,c);
        queue.invalidate();
        queue.completed();
        assert_eq!(queue.damage(1,output,b),output);
        queue.submitted(1,b);queue.completed();
        assert_eq!(queue.next(),None);
        queue.invalidate_cursor();
        assert_eq!(queue.damage(0,output,c),output);
        queue.submitted(0,c);queue.completed();
        queue.forget_damage_history();
        assert_eq!(queue.next(),None);
        queue.invalidate_cursor();
        assert_eq!(queue.damage(1,output,a),output);
    }
    #[test]
    fn idle_and_unexpected_completion_do_not_submit() {
        let mut queue = FrameQueue::default();
        assert_eq!(queue.completed(), None);
        queue.submitted(0, cursor(0,0));
        queue.completed();
        assert_eq!(queue.next(), None);
        assert_eq!(queue.completed(), None);
    }
}
