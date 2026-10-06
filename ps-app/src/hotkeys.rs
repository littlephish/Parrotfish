use std::time::{Duration, Instant};

pub const MAX_CHORD_KEYS: usize = 4;
pub const MAX_WHISPER_KEYS: usize = 12;
pub const REPLY_LANE: u8 = 13;
const ESCAPE: u16 = 0x1B;

pub fn usable(vk: u16) -> bool {
    (3..=254).contains(&vk) && !matches!(vk, 0x10 | 0x11 | 0x12)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Chord(Vec<u16>);

impl Chord {
    pub fn new(keys: &[u16]) -> Self {
        let mut list: Vec<u16> = keys.iter().copied().filter(|vk| usable(*vk)).collect();
        list.sort_unstable();
        list.dedup();
        list.truncate(MAX_CHORD_KEYS);
        Self(list)
    }

    pub fn keys(&self) -> &[u16] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn parse(text: &str) -> Self {
        let keys: Vec<u16> = text.split('+').filter_map(|part| part.trim().parse::<u16>().ok()).collect();
        Self::new(&keys)
    }

    pub fn to_text(&self) -> String {
        self.0.iter().map(|vk| vk.to_string()).collect::<Vec<String>>().join("+")
    }

    pub fn is_down(&self, down: &dyn Fn(u16) -> bool) -> bool {
        !self.0.is_empty() && self.0.iter().all(|vk| down(*vk))
    }
}

pub fn key_name(vk: u16, character: &dyn Fn(u16) -> Option<char>) -> String {
    let fixed = match vk {
        0x04 => "Mouse 3",
        0x05 => "Mouse 4",
        0x06 => "Mouse 5",
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0D => "Enter",
        0x13 => "Pause",
        0x14 => "Caps Lock",
        0x1B => "Esc",
        0x20 => "Space",
        0x21 => "Page Up",
        0x22 => "Page Down",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x2C => "Print Screen",
        0x2D => "Insert",
        0x2E => "Delete",
        0x5B => "Left Win",
        0x5C => "Right Win",
        0x5D => "Menu",
        0x6A => "Num *",
        0x6B => "Num +",
        0x6D => "Num -",
        0x6E => "Num .",
        0x6F => "Num /",
        0x90 => "Num Lock",
        0x91 => "Scroll Lock",
        0xA0 => "Left Shift",
        0xA1 => "Right Shift",
        0xA2 => "Left Ctrl",
        0xA3 => "Right Ctrl",
        0xA4 => "Left Alt",
        0xA5 => "Right Alt",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.to_string();
    }
    if (0x60..=0x69).contains(&vk) {
        return format!("Num {}", vk - 0x60);
    }
    if (0x70..=0x87).contains(&vk) {
        return format!("F{}", vk - 0x6F);
    }
    match character(vk) {
        Some(c) if !c.is_control() && !c.is_whitespace() => c.to_uppercase().collect(),
        _ => format!("Key {vk}"),
    }
}

fn rank(vk: u16) -> u8 {
    match vk {
        0xA2 | 0xA3 => 0,
        0xA0 | 0xA1 => 1,
        0xA4 | 0xA5 => 2,
        0x5B | 0x5C => 3,
        _ => 4,
    }
}

pub fn chord_name(chord: &Chord, character: &dyn Fn(u16) -> Option<char>) -> String {
    let mut keys = chord.keys().to_vec();
    keys.sort_by_key(|vk| (rank(*vk), *vk));
    keys.iter().map(|vk| key_name(*vk, character)).collect::<Vec<String>>().join(" + ")
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bindings {
    pub talk: Vec<Chord>,
    pub whisper: Vec<Chord>,
    pub reply: Chord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Held {
    pub talk: bool,
    pub lane: u8,
}

pub fn evaluate(bindings: &Bindings, down: &dyn Fn(u16) -> bool) -> Held {
    let talk = bindings.talk.iter().any(|chord| chord.is_down(down));
    let mut best: Option<(usize, u8)> = None;
    for (index, chord) in bindings.whisper.iter().enumerate().take(MAX_WHISPER_KEYS) {
        if chord.is_down(down) && best.map_or(true, |(size, _)| chord.keys().len() > size) {
            best = Some((chord.keys().len(), index as u8 + 1));
        }
    }
    if bindings.reply.is_down(down) && best.map_or(true, |(size, _)| bindings.reply.keys().len() > size) {
        best = Some((bindings.reply.keys().len(), REPLY_LANE));
    }
    Held { talk, lane: best.map_or(0, |(_, lane)| lane) }
}

#[derive(Debug, Default)]
pub struct Latch {
    talk: bool,
    talk_until: Option<Instant>,
    lane: u8,
    lane_until: Option<Instant>,
}

impl Latch {
    pub fn update(&mut self, now: Instant, raw: Held, delay: Duration) -> Held {
        if raw.lane != 0 {
            self.lane = raw.lane;
            self.lane_until = None;
        } else if self.lane != 0 {
            let until = *self.lane_until.get_or_insert(now + delay);
            if now >= until {
                self.lane = 0;
                self.lane_until = None;
            }
        }
        if raw.talk {
            self.talk = true;
            self.talk_until = None;
        } else if self.talk {
            let until = *self.talk_until.get_or_insert(now + delay);
            if now >= until {
                self.talk = false;
                self.talk_until = None;
            }
        }
        Held { talk: self.talk, lane: self.lane }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureStep {
    Waiting,
    Done(Chord),
    Cancelled,
}

#[derive(Debug, Default)]
pub struct Capture {
    armed: bool,
    seen: Vec<u16>,
}

impl Capture {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, down: &[u16]) -> CaptureStep {
        if !self.armed {
            self.armed = down.is_empty();
            return CaptureStep::Waiting;
        }
        if down.is_empty() {
            return if self.seen.is_empty() { CaptureStep::Waiting } else { CaptureStep::Done(Chord::new(&self.seen)) };
        }
        if self.seen.is_empty() && down.len() == 1 && down[0] == ESCAPE {
            return CaptureStep::Cancelled;
        }
        for vk in down {
            if !self.seen.contains(vk) {
                self.seen.push(*vk);
            }
        }
        CaptureStep::Waiting
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn down_set(keys: &[u16]) -> impl Fn(u16) -> bool + '_ {
        move |vk| keys.contains(&vk)
    }

    #[test]
    fn chords_are_normalised_and_round_trip() {
        let chord = Chord::new(&[0x41, 0xA2, 0x41, 0x11, 0x01]);
        assert_eq!(chord.keys(), &[0x41, 0xA2]);
        assert_eq!(chord.to_text(), "65+162");
        assert_eq!(Chord::parse("65+162"), chord);
        assert_eq!(Chord::parse(" 162 + 65 "), chord);
        assert!(Chord::parse("").is_empty());
        assert!(Chord::parse("abc+0+999+1+2").is_empty());
        assert_eq!(Chord::parse("135+134+133+132+131").keys().len(), MAX_CHORD_KEYS);
        assert!(!Chord::default().is_down(&|_| true));
        assert!(chord.is_down(&down_set(&[0x41, 0xA2, 0x20])));
        assert!(!chord.is_down(&down_set(&[0xA2])));
    }

    #[test]
    fn keys_have_readable_names() {
        let letters = |vk: u16| match vk {
            0x30..=0x5A => char::from_u32(u32::from(vk)),
            0xC0 => Some('`'),
            _ => None,
        };
        assert_eq!(key_name(0xA2, &letters), "Left Ctrl");
        assert_eq!(key_name(0xA5, &letters), "Right Alt");
        assert_eq!(key_name(0x04, &letters), "Mouse 3");
        assert_eq!(key_name(0x05, &letters), "Mouse 4");
        assert_eq!(key_name(0x2D, &letters), "Insert");
        assert_eq!(key_name(0x13, &letters), "Pause");
        assert_eq!(key_name(0x65, &letters), "Num 5");
        assert_eq!(key_name(0x7C, &letters), "F13");
        assert_eq!(key_name(0x87, &letters), "F24");
        assert_eq!(key_name(0x41, &letters), "A");
        assert_eq!(key_name(0xC0, &letters), "`");
        assert_eq!(key_name(0xE8, &letters), "Key 232");
        assert_eq!(chord_name(&Chord::new(&[0x41, 0xA0, 0xA2]), &letters), "Left Ctrl + Left Shift + A");
        assert_eq!(chord_name(&Chord::default(), &letters), "");
    }

    #[test]
    fn the_most_specific_whisper_key_wins() {
        let bindings = Bindings {
            talk: vec![Chord::new(&[0xA2]), Chord::new(&[0x05])],
            whisper: vec![Chord::new(&[0x65]), Chord::new(&[0xA2, 0x31]), Chord::new(&[0x31])],
            reply: Chord::new(&[0x60]),
        };
        assert_eq!(evaluate(&bindings, &down_set(&[])), Held { talk: false, lane: 0 });
        assert_eq!(evaluate(&bindings, &down_set(&[0xA2])), Held { talk: true, lane: 0 });
        assert_eq!(evaluate(&bindings, &down_set(&[0x05])), Held { talk: true, lane: 0 });
        assert_eq!(evaluate(&bindings, &down_set(&[0x65])), Held { talk: false, lane: 1 });
        assert_eq!(evaluate(&bindings, &down_set(&[0xA2, 0x31])), Held { talk: true, lane: 2 });
        assert_eq!(evaluate(&bindings, &down_set(&[0x31])), Held { talk: false, lane: 3 });
        assert_eq!(evaluate(&bindings, &down_set(&[0x65, 0x31])), Held { talk: false, lane: 1 });
        assert_eq!(evaluate(&bindings, &down_set(&[0x60])), Held { talk: false, lane: REPLY_LANE });
        assert_eq!(evaluate(&Bindings::default(), &down_set(&[0xA2, 0x65])), Held::default());
    }

    #[test]
    fn release_is_delayed_but_presses_are_not() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let delay = Duration::from_millis(200);
        let talk = Held { talk: true, lane: 0 };
        let none = Held::default();
        let lane = |n: u8| Held { talk: false, lane: n };
        let mut latch = Latch::default();
        assert_eq!(latch.update(at(0), talk, delay), talk);
        assert_eq!(latch.update(at(100), none, delay), talk);
        assert_eq!(latch.update(at(299), none, delay), talk);
        assert_eq!(latch.update(at(300), none, delay), none);
        assert_eq!(latch.update(at(400), lane(2), delay), lane(2));
        assert_eq!(latch.update(at(410), lane(3), delay), lane(3));
        assert_eq!(latch.update(at(420), none, delay), lane(3));
        assert_eq!(latch.update(at(500), lane(3), delay), lane(3));
        assert_eq!(latch.update(at(600), none, delay), lane(3));
        assert_eq!(latch.update(at(800), none, delay), none);
        let mut instant = Latch::default();
        assert_eq!(instant.update(at(0), talk, Duration::ZERO), talk);
        assert_eq!(instant.update(at(1), none, Duration::ZERO), none);
    }

    #[test]
    fn capture_waits_for_a_clean_start_and_a_full_release() {
        let mut capture = Capture::new();
        assert_eq!(capture.feed(&[0xA2]), CaptureStep::Waiting);
        assert_eq!(capture.feed(&[]), CaptureStep::Waiting);
        assert_eq!(capture.feed(&[0xA2]), CaptureStep::Waiting);
        assert_eq!(capture.feed(&[0xA2, 0x41]), CaptureStep::Waiting);
        assert_eq!(capture.feed(&[0x41]), CaptureStep::Waiting);
        assert_eq!(capture.feed(&[]), CaptureStep::Done(Chord::new(&[0xA2, 0x41])));

        let mut cancelled = Capture::new();
        cancelled.feed(&[]);
        assert_eq!(cancelled.feed(&[0x1B]), CaptureStep::Cancelled);

        let mut with_escape = Capture::new();
        with_escape.feed(&[]);
        with_escape.feed(&[0xA2]);
        with_escape.feed(&[0xA2, 0x1B]);
        assert_eq!(with_escape.feed(&[]), CaptureStep::Done(Chord::new(&[0xA2, 0x1B])));

        let mut never_armed = Capture::new();
        assert_eq!(never_armed.feed(&[0x41]), CaptureStep::Waiting);
        assert_eq!(never_armed.feed(&[0x41]), CaptureStep::Waiting);
    }
}
