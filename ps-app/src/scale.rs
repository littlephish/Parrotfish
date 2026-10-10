pub type Place = (i32, i32, i32, i32);

#[derive(Debug, Default)]
pub struct ScaleWatch {
    seen: Option<f32>,
    nudged: bool,
}

impl ScaleWatch {
    pub fn changed(&mut self, scale: f32) -> bool {
        if scale <= 0.0 {
            return false;
        }
        let changed = self.seen.is_some_and(|seen| (seen - scale).abs() > 0.005);
        self.seen = Some(scale);
        if changed {
            self.nudged = !self.nudged;
        }
        changed
    }

    pub fn nudge(&self) -> f32 {
        if self.nudged {
            1.0
        } else {
            0.0
        }
    }
}

#[derive(Debug, Default)]
pub struct Landing {
    dragged: bool,
    suggested: Vec<Option<Place>>,
}

impl Landing {
    pub fn drag(&mut self, on: bool) {
        self.dragged = on;
    }

    pub fn scale_changes(&mut self, suggested: Place) {
        self.suggested.push(self.dragged.then_some(suggested));
    }

    pub fn scale_changed(&mut self) {
        self.suggested.pop();
    }

    pub fn place(&mut self, asked: Place, whole: bool) -> Place {
        match self.suggested.last_mut() {
            Some(waiting) if whole => waiting.take().unwrap_or(asked),
            _ => asked,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limits_are_worked_out_again_when_the_display_scale_changes() {
        let mut watch = ScaleWatch::default();
        assert!(!watch.changed(1.5), "the first scale seen is where the window starts");
        assert!(!watch.changed(1.5));
        assert_eq!(watch.nudge(), 0.0);
        assert!(watch.changed(1.0));
        assert_eq!(watch.nudge(), 1.0);
        assert!(!watch.changed(1.0));
        assert!(watch.changed(1.5));
        assert_eq!(watch.nudge(), 0.0, "each change moves the limits, so they are always applied afresh");
        assert!(!watch.changed(0.0), "a scale that cannot be is not a change");
        assert!(!watch.changed(-1.0));
        assert!(!watch.changed(1.5));
    }

    #[test]
    fn a_window_dragged_to_another_scale_lands_where_windows_suggests() {
        let toolkit = (3655, 75, 401, 1112);
        let windows = (3684, 75, 400, 1111);
        let mut landing = Landing::default();
        landing.drag(true);
        landing.scale_changes(windows);
        assert_eq!(landing.place(toolkit, true), windows);
        assert_eq!(landing.place((0, 0, 0, 0), false), (0, 0, 0, 0), "a placement that keeps the size or the place is left alone");
        assert_eq!(landing.place(toolkit, true), toolkit, "only the first placement of a scale change is moved");
        landing.scale_changed();
        assert_eq!(landing.place(toolkit, true), toolkit, "afterwards the window goes where it is put");
    }

    #[test]
    fn only_a_dragged_window_is_moved_to_the_suggestion() {
        let toolkit = (3650, 68, 401, 1112);
        let windows = (3650, 68, 400, 1111);
        let mut landing = Landing::default();
        landing.scale_changes(windows);
        assert_eq!(landing.place(toolkit, true), toolkit);
        landing.scale_changed();
        landing.drag(true);
        landing.drag(false);
        landing.scale_changes(windows);
        assert_eq!(landing.place(toolkit, true), toolkit, "the drag was over");
        landing.scale_changed();
    }

    #[test]
    fn a_scale_change_inside_another_has_its_own_suggestion() {
        let (outer, inner) = ((3684, 75, 400, 1111), (3582, 75, 600, 1666));
        let mut landing = Landing::default();
        landing.drag(true);
        landing.scale_changes(outer);
        landing.scale_changes(inner);
        assert_eq!(landing.place((1, 2, 3, 4), true), inner);
        landing.scale_changed();
        assert_eq!(landing.place((1, 2, 3, 4), true), outer, "the outer change has not placed the window yet");
        landing.scale_changed();
        landing.scale_changed();
        assert_eq!(landing.place((1, 2, 3, 4), true), (1, 2, 3, 4), "one end too many does no harm");
    }
}
