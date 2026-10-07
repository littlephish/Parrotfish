#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Step {
    Nothing,
    Refresh,
    Restore(f32, f32),
}

#[derive(Debug, Default)]
pub struct ScaleWatch {
    seen: Option<(f32, f32, f32)>,
    pending: Option<(f32, f32)>,
    nudged: bool,
}

impl ScaleWatch {
    pub fn step(&mut self, scale: f32, width: f32, height: f32, free: bool) -> Step {
        if width < 1.0 || height < 1.0 || scale <= 0.0 {
            return Step::Nothing;
        }
        if let Some((wanted_width, wanted_height)) = self.pending.take() {
            if free {
                self.seen = Some((scale, wanted_width, wanted_height));
                return Step::Restore(wanted_width, wanted_height);
            }
            self.seen = Some((scale, width, height));
            return Step::Nothing;
        }
        match self.seen {
            Some((seen, kept_width, kept_height)) if (seen - scale).abs() > 0.005 => {
                self.pending = Some((kept_width, kept_height));
                self.seen = Some((scale, kept_width, kept_height));
                self.nudged = !self.nudged;
                Step::Refresh
            }
            _ => {
                self.seen = Some((scale, width, height));
                Step::Nothing
            }
        }
    }

    pub fn nudge(&self) -> f32 {
        if self.nudged {
            1.0
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_keeps_its_size_when_the_display_scale_changes() {
        let mut watch = ScaleWatch::default();
        assert_eq!(watch.step(1.5, 400.0, 740.0, true), Step::Nothing);
        assert_eq!(watch.step(1.5, 420.0, 700.0, true), Step::Nothing);
        assert_eq!(watch.nudge(), 0.0);
        assert_eq!(watch.step(1.0, 510.0, 780.0, true), Step::Refresh, "the limits are worked out again first");
        assert_eq!(watch.nudge(), 1.0);
        assert_eq!(watch.step(1.0, 510.0, 780.0, true), Step::Restore(420.0, 700.0));
        assert_eq!(watch.step(1.0, 420.0, 700.0, true), Step::Nothing);
        assert_eq!(watch.step(1.0, 450.0, 710.0, true), Step::Nothing, "sizing it by hand is left alone");
        assert_eq!(watch.step(1.5, 450.0, 710.0, true), Step::Refresh);
        assert_eq!(watch.nudge(), 0.0, "each change moves the limits, so they are always applied afresh");
        assert_eq!(watch.step(1.5, 450.0, 710.0, true), Step::Restore(450.0, 710.0));
    }

    #[test]
    fn a_maximised_or_hidden_window_is_not_resized() {
        let mut watch = ScaleWatch::default();
        assert_eq!(watch.step(1.0, 1920.0, 1040.0, false), Step::Nothing);
        assert_eq!(watch.step(1.5, 1280.0, 693.0, false), Step::Refresh);
        assert_eq!(watch.step(1.5, 1280.0, 693.0, false), Step::Nothing);
        assert_eq!(watch.step(1.5, 0.0, 0.0, true), Step::Nothing, "a minimised window reports no size");
        assert_eq!(watch.step(2.0, 0.0, 0.0, true), Step::Nothing);
        assert_eq!(watch.step(2.0, 960.0, 520.0, true), Step::Refresh);
        assert_eq!(watch.step(2.0, 960.0, 520.0, true), Step::Restore(1280.0, 693.0));
        assert_eq!(watch.step(2.0, 0.0, 520.0, true), Step::Nothing);
        assert_eq!(watch.step(-1.0, 960.0, 520.0, true), Step::Nothing);
    }
}
