//! Panels that open and close, and the easing that gets them there.
//!
//! GPUI has no transitions, so a width that changes has to be walked to its new
//! value a frame at a time. Each step covers a fraction of what is left, which
//! is an ease-out: fast where the movement is obvious and slow where it lands.
//! It also makes an interruption free — changing the target mid-flight is just
//! a different number to walk towards, with no animation to cancel.

/// How much of the remaining distance a pane covers each frame.
const EASING: f32 = 0.28;

/// Below this, the difference is not worth another frame.
const SETTLED: f32 = 0.5;

/// A panel that slides open and shut.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pane {
    /// What it is now, in logical pixels.
    pub width: f32,
    /// What it is heading for.
    pub target: f32,
    /// How wide it is when open. Kept so that opening does not need to be told.
    pub open_width: f32,
}

impl Pane {
    pub fn open(open_width: f32) -> Self {
        Self {
            width: open_width,
            target: open_width,
            open_width,
        }
    }

    pub fn shut(open_width: f32) -> Self {
        Self {
            width: 0.0,
            target: 0.0,
            open_width,
        }
    }

    /// Whether it is open, or on its way there.
    pub fn is_open(&self) -> bool {
        self.target > 0.0
    }

    pub fn set_open(&mut self, open: bool) {
        self.target = if open { self.open_width } else { 0.0 };
    }

    pub fn toggle(&mut self) {
        self.set_open(!self.is_open());
    }

    /// Whether it is drawn at all. A pane of no width has nothing to show and
    /// its contents should not be laid out.
    pub fn is_visible(&self) -> bool {
        self.width > SETTLED
    }

    /// Moves one frame towards the target. Returns whether it has further to
    /// go.
    pub fn step(&mut self) -> bool {
        if (self.target - self.width).abs() <= SETTLED {
            self.width = self.target;
            return false;
        }

        self.width += (self.target - self.width) * EASING;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shut_pane_is_not_drawn() {
        assert!(!Pane::shut(300.0).is_visible());
        assert!(Pane::open(300.0).is_visible());
    }

    #[test]
    fn stepping_arrives_and_then_stops() {
        let mut pane = Pane::open(300.0);
        pane.set_open(false);

        let mut frames = 0;
        while pane.step() {
            frames += 1;
            assert!(frames < 200, "it never arrived");
        }

        assert_eq!(pane.width, 0.0);
        assert!(!pane.step(), "it kept going after arriving");
    }

    /// An ease-out: the first frame moves further than the last.
    #[test]
    fn it_slows_down_as_it_lands() {
        let mut pane = Pane::shut(300.0);
        pane.set_open(true);

        let start = pane.width;
        pane.step();
        let first = pane.width - start;

        for _ in 0..10 {
            pane.step();
        }
        let before = pane.width;
        pane.step();

        assert!(pane.width - before < first, "it did not slow down");
    }

    /// Changing your mind mid-slide is a different target, not a cancellation.
    #[test]
    fn it_can_turn_around_halfway() {
        let mut pane = Pane::open(300.0);
        pane.set_open(false);
        for _ in 0..3 {
            pane.step();
        }

        let halfway = pane.width;
        assert!(halfway > 0.0 && halfway < 300.0, "{halfway}");

        pane.set_open(true);
        while pane.step() {}
        assert_eq!(pane.width, 300.0);
    }

    #[test]
    fn a_pane_knows_where_it_is_going_before_it_arrives() {
        let mut pane = Pane::shut(300.0);
        pane.set_open(true);

        assert!(
            pane.is_open(),
            "it should count as open the moment it is asked"
        );
    }
}
