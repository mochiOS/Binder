//! Join the first shell frame and initial background attempt, in either order.
#[derive(Default)]
pub(super) struct Gate {
    frame: bool,
    background: bool,
    published: bool,
}
impl Gate {
    pub(super) fn frame_presented(&mut self) -> bool {
        self.frame = true;
        self.advance()
    }
    pub(super) fn background_finished(&mut self) -> bool {
        self.background = true;
        self.advance()
    }
    fn advance(&mut self) -> bool {
        let ready = self.frame && self.background && !self.published;
        self.published |= ready;
        ready
    }
}
#[cfg(test)]
mod tests {
    use super::Gate;
    #[test]
    fn first_frame_alone_is_not_a_completed_desktop() {
        let mut gate = Gate::default();
        assert!(!gate.frame_presented());
        assert!(!gate.frame_presented());
        assert!(gate.background_finished());
        assert!(!gate.background_finished());
        assert!(!gate.frame_presented());
    }
    #[test]
    fn preloaded_background_waits_for_the_shell_frame() {
        let mut gate = Gate::default();
        assert!(!gate.background_finished());
        assert!(!gate.background_finished());
        assert!(gate.frame_presented());
        assert!(!gate.frame_presented());
        assert!(!gate.background_finished());
    }
}
