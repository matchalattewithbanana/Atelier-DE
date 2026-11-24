//! Two-slot scanout scheduling independent of DRM and rendering.
#[derive(Debug)]
pub struct FrameQueue {
    dirty: bool,
    displayed: Option<usize>,
    pending: Option<usize>,
}

impl Default for FrameQueue {
    fn default() -> Self { Self { dirty: true, displayed: None, pending: None } }
}

impl FrameQueue {
    pub fn invalidate(&mut self) { self.dirty = true; }
    pub fn displayed(&self) -> Option<usize> { self.displayed }
    pub fn pending(&self) -> Option<usize> { self.pending }
    pub fn next(&self) -> Option<usize> {
        if !self.dirty || self.pending.is_some() { return None; }
        Some(match self.displayed { Some(0) => 1, _ => 0 })
    }
    /// Call only after a successful kernel submission of the slot from next().
    pub fn submitted(&mut self, slot: usize) {
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
    #[test]
    fn never_reuses_displayed_or_pending_slot() {
        let mut queue = FrameQueue::default();
        assert_eq!(queue.next(), Some(0));
        queue.submitted(0);
        queue.invalidate();
        assert_eq!(queue.next(), None);
        assert_eq!(queue.completed(), Some(0));
        assert_eq!(queue.next(), Some(1));
        queue.submitted(1);
        assert_eq!(queue.next(), None);
        assert_eq!(queue.completed(), Some(1));
        assert_eq!(queue.next(), None);
        queue.invalidate();
        assert_eq!(queue.next(), Some(0));
    }

    #[test]
    fn idle_and_unexpected_completion_do_not_submit() {
        let mut queue = FrameQueue::default();
        assert_eq!(queue.completed(), None);
        queue.submitted(0);
        queue.completed();
        assert_eq!(queue.next(), None);
        assert_eq!(queue.completed(), None);
    }
}
