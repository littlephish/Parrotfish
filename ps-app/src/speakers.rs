use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub const DEFAULT_LINGER_SECONDS: u32 = 10;
pub const MAX_LINGER_SECONDS: u32 = 60;
pub const ROW_HEIGHT: f32 = 22.0;
pub const PADDING: f32 = 8.0;
pub const MIN_WIDTH: f32 = 120.0;
pub const MIN_HEIGHT: f32 = 38.0;
pub const MAX_SIDE: f32 = 2000.0;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Member {
    pub id: u16,
    pub name: String,
    pub talking: bool,
    pub whispering: bool,
    pub me: bool,
    pub here: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Room {
    pub session: u16,
    pub name: String,
    pub members: Vec<Member>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Line {
    pub caption: bool,
    pub text: String,
    pub talking: bool,
    pub whispering: bool,
    pub me: bool,
    pub extra: usize,
}

#[derive(Debug, Clone, Copy)]
struct Heard {
    stopped: Option<Instant>,
    whispered: bool,
}

#[derive(Debug, Default)]
pub struct Roster {
    heard: HashMap<(u16, u16), Heard>,
    due: Option<Instant>,
}

pub fn linger(seconds: u32) -> Duration {
    Duration::from_secs(u64::from(seconds.min(MAX_LINGER_SECONDS)))
}

impl Roster {
    pub fn lines(&mut self, rooms: &[Room], everyone: bool, linger: Duration, now: Instant) -> Vec<Line> {
        let mut kept = HashMap::new();
        let mut due: Option<Instant> = None;
        let mut lines = Vec::new();
        for room in rooms {
            let mut people = Vec::new();
            for member in &room.members {
                let key = (room.session, member.id);
                let mut whispering = member.talking && member.whispering;
                let mut lingering = false;
                if member.talking {
                    kept.insert(key, Heard { stopped: None, whispered: member.whispering });
                } else if let Some(before) = self.heard.get(&key) {
                    let stopped = before.stopped.unwrap_or(now);
                    if now.saturating_duration_since(stopped) < linger {
                        kept.insert(key, Heard { stopped: Some(stopped), whispered: before.whispered });
                        whispering = before.whispered;
                        lingering = true;
                        let ends = stopped + linger;
                        due = Some(due.map_or(ends, |soonest| soonest.min(ends)));
                    }
                }
                if member.talking || lingering || (everyone && member.here) {
                    people.push(Line {
                        text: member.name.clone(),
                        talking: member.talking,
                        whispering,
                        me: member.me,
                        ..Line::default()
                    });
                }
            }
            if rooms.len() > 1 && !people.is_empty() {
                lines.push(Line { caption: true, text: room.name.clone(), ..Line::default() });
            }
            lines.extend(people);
        }
        self.heard = kept;
        self.due = due;
        lines
    }

    pub fn due(&self, now: Instant) -> bool {
        self.due.is_some_and(|at| now >= at)
    }
}

pub fn capacity(height: f32) -> usize {
    ((height - 2.0 * PADDING) / ROW_HEIGHT).floor().max(0.0) as usize
}

fn arrange(lines: &[Line], chosen: &HashSet<usize>) -> Vec<Line> {
    let mut shown = Vec::new();
    let mut heading: Option<&Line> = None;
    for (index, line) in lines.iter().enumerate() {
        if line.caption {
            heading = Some(line);
        } else if chosen.contains(&index) {
            if let Some(heading) = heading.take() {
                shown.push(heading.clone());
            }
            shown.push(line.clone());
        }
    }
    shown
}

pub fn fit(lines: Vec<Line>, room: usize) -> Vec<Line> {
    if lines.len() <= room {
        return lines;
    }
    let people = |talking: bool| {
        lines.iter().enumerate().filter(move |(_, line)| !line.caption && line.talking == talking).map(|(index, _)| index)
    };
    let order: Vec<usize> = people(true).chain(people(false)).collect();
    let mut keep = order.len().min(room);
    while keep > 0 {
        let chosen: HashSet<usize> = order[..keep].iter().copied().collect();
        let mut shown = arrange(&lines, &chosen);
        if shown.len() <= room {
            if let Some(last) = shown.iter_mut().rev().find(|line| !line.caption) {
                last.extra = order.len() - keep;
            }
            return shown;
        }
        keep -= 1;
    }
    if room == 0 || order.is_empty() {
        return Vec::new();
    }
    fit(lines.into_iter().filter(|line| !line.caption).collect(), room)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINGER: Duration = Duration::from_secs(3);

    fn member(id: u16, name: &str, talking: bool) -> Member {
        Member { id, name: name.to_string(), talking, here: true, ..Member::default() }
    }

    fn room(session: u16, name: &str, members: Vec<Member>) -> Room {
        Room { session, name: name.to_string(), members }
    }

    fn person(text: &str, talking: bool) -> Line {
        Line { text: text.to_string(), talking, ..Line::default() }
    }

    fn heading(text: &str) -> Line {
        Line { caption: true, text: text.to_string(), ..Line::default() }
    }

    fn and_more(text: &str, extra: usize) -> Line {
        Line { text: text.to_string(), talking: true, extra, ..Line::default() }
    }

    #[test]
    fn people_are_listed_while_they_talk_and_a_moment_longer() {
        let start = Instant::now();
        let mut roster = Roster::default();
        let talking = [room(1, "Reef", vec![member(1, "Minnow", false), member(2, "Pike", true), member(3, "Bream", false)])];
        assert_eq!(roster.lines(&talking, false, LINGER, start), vec![person("Pike", true)]);
        assert!(!roster.due(start + Duration::from_secs(60)), "nothing runs out while somebody talks");

        let quiet = [room(1, "Reef", vec![member(1, "Minnow", false), member(2, "Pike", false), member(3, "Bream", false)])];
        let later = start + Duration::from_secs(1);
        assert_eq!(roster.lines(&quiet, false, LINGER, later), vec![person("Pike", false)]);
        assert!(!roster.due(later));
        assert!(!roster.due(later + LINGER - Duration::from_millis(1)));
        assert!(roster.due(later + LINGER));
        assert_eq!(roster.lines(&quiet, false, LINGER, later + Duration::from_secs(2)), vec![person("Pike", false)]);
        assert_eq!(roster.lines(&quiet, false, LINGER, later + LINGER), Vec::new());
        assert!(!roster.due(start + Duration::from_secs(60)));
    }

    #[test]
    fn the_wait_starts_when_they_stop_however_long_they_talked() {
        let start = Instant::now();
        let mut roster = Roster::default();
        let on = [room(1, "Reef", vec![member(2, "Pike", true)])];
        let off = [room(1, "Reef", vec![member(2, "Pike", false)])];
        roster.lines(&on, false, LINGER, start);
        let stop = start + Duration::from_secs(30);
        assert_eq!(roster.lines(&off, false, LINGER, stop), vec![person("Pike", false)]);
        assert!(!roster.due(stop + Duration::from_secs(2)));
        assert_eq!(roster.lines(&off, false, LINGER, stop + Duration::from_secs(2)), vec![person("Pike", false)]);
        assert!(roster.due(stop + LINGER));
        assert_eq!(roster.lines(&off, false, LINGER, stop + LINGER), Vec::new());
    }

    #[test]
    fn talking_again_starts_the_wait_afresh() {
        let start = Instant::now();
        let mut roster = Roster::default();
        let on = [room(1, "Reef", vec![member(2, "Pike", true)])];
        let off = [room(1, "Reef", vec![member(2, "Pike", false)])];
        roster.lines(&on, false, LINGER, start);
        roster.lines(&off, false, LINGER, start + Duration::from_secs(2));
        roster.lines(&on, false, LINGER, start + Duration::from_millis(2500));
        let late = start + Duration::from_secs(5);
        assert_eq!(roster.lines(&off, false, LINGER, late), vec![person("Pike", false)]);
        assert!(!roster.due(late));
        assert!(!roster.due(start + Duration::from_millis(5500)));
        assert!(roster.due(late + LINGER));
    }

    #[test]
    fn the_wait_can_be_changed_or_turned_off() {
        let start = Instant::now();
        let on = [room(1, "Reef", vec![member(2, "Pike", true)])];
        let off = [room(1, "Reef", vec![member(2, "Pike", false)])];
        let mut roster = Roster::default();
        roster.lines(&on, false, Duration::ZERO, start);
        assert_eq!(roster.lines(&off, false, Duration::ZERO, start), Vec::new());
        assert!(!roster.due(start + Duration::from_secs(60)));

        let long = linger(10);
        let mut roster = Roster::default();
        roster.lines(&on, false, long, start);
        assert_eq!(roster.lines(&off, false, long, start + Duration::from_secs(9)), vec![person("Pike", false)]);
        assert!(!roster.due(start + Duration::from_secs(18)));
        assert!(roster.due(start + Duration::from_secs(19)));
        assert_eq!(roster.lines(&off, false, long, start + Duration::from_secs(19)), Vec::new());
        assert_eq!(linger(0), Duration::ZERO);
        assert_eq!(linger(DEFAULT_LINGER_SECONDS), Duration::from_secs(10));
        assert_eq!(linger(5000), Duration::from_secs(60));
    }

    #[test]
    fn everyone_in_my_channel_can_be_listed() {
        let now = Instant::now();
        let mut roster = Roster::default();
        let mut me = member(1, "Minnow", true);
        me.me = true;
        let mut elsewhere = member(4, "Carp", false);
        elsewhere.here = false;
        let rooms = [room(1, "Reef", vec![me, member(2, "Pike", false), elsewhere])];
        let mut mine = person("Minnow", true);
        mine.me = true;
        assert_eq!(roster.lines(&rooms, true, LINGER, now), vec![mine.clone(), person("Pike", false)]);
        assert!(!roster.due(now + Duration::from_secs(60)), "a list of everyone has nothing that runs out");
        assert_eq!(Roster::default().lines(&rooms, false, LINGER, now), vec![mine]);
    }

    #[test]
    fn a_whisper_from_another_channel_is_marked_and_stays_marked() {
        let start = Instant::now();
        let mut roster = Roster::default();
        let mut carp = member(4, "Carp", true);
        carp.here = false;
        carp.whispering = true;
        let rooms = [room(1, "Reef", vec![member(2, "Pike", false), carp.clone()])];
        let mut marked = person("Carp", true);
        marked.whispering = true;
        assert_eq!(roster.lines(&rooms, false, LINGER, start), vec![marked.clone()]);

        carp.talking = false;
        carp.whispering = false;
        let after = [room(1, "Reef", vec![member(2, "Pike", false), carp])];
        marked.talking = false;
        let stop = start + Duration::from_secs(1);
        assert_eq!(roster.lines(&after, false, LINGER, stop), vec![marked.clone()]);
        assert_eq!(roster.lines(&after, true, LINGER, stop + Duration::from_secs(2)), vec![person("Pike", false), marked]);
        assert_eq!(roster.lines(&after, true, LINGER, stop + LINGER), vec![person("Pike", false)]);
    }

    #[test]
    fn several_servers_get_a_heading_each_and_quiet_ones_none() {
        let now = Instant::now();
        let mut roster = Roster::default();
        let rooms = [
            room(1, "Reef", vec![member(2, "Pike", true)]),
            room(2, "Night Shift", vec![member(2, "Pike", false)]),
            room(3, "Harbour", vec![member(7, "Eel", true)]),
        ];
        assert_eq!(
            roster.lines(&rooms, false, LINGER, now),
            vec![heading("Reef"), person("Pike", true), heading("Harbour"), person("Eel", true)]
        );
        let later = now + Duration::from_secs(1);
        assert_eq!(roster.lines(&rooms[1..2], false, LINGER, later), Vec::new(), "the same number on another server is someone else");
    }

    #[test]
    fn someone_who_left_is_forgotten() {
        let start = Instant::now();
        let mut roster = Roster::default();
        roster.lines(&[room(1, "Reef", vec![member(2, "Pike", true)])], false, LINGER, start);
        let later = start + Duration::from_secs(1);
        assert_eq!(roster.lines(&[room(1, "Reef", vec![])], false, LINGER, later), Vec::new());
        assert!(!roster.due(start + Duration::from_secs(60)));
        let back = [room(1, "Reef", vec![member(2, "Pike", false)])];
        assert_eq!(roster.lines(&back, false, LINGER, later), Vec::new());
    }

    #[test]
    fn the_height_decides_how_many_lines_fit() {
        assert_eq!(capacity(160.0), 6);
        assert_eq!(capacity(38.0), 1);
        assert_eq!(capacity(37.0), 0);
        assert_eq!(capacity(0.0), 0);
        assert_eq!(capacity(-5.0), 0);
        assert_eq!(capacity(MIN_HEIGHT), 1);
    }

    #[test]
    fn a_short_window_keeps_the_people_who_talk() {
        let lines = vec![
            person("Minnow", false),
            person("Pike", false),
            person("Bream", true),
            person("Carp", false),
            person("Eel", true),
        ];
        assert_eq!(fit(lines.clone(), 5), lines);
        assert_eq!(fit(lines.clone(), 9), lines);
        assert_eq!(
            fit(lines.clone(), 4),
            vec![person("Minnow", false), person("Pike", false), person("Bream", true), and_more("Eel", 1)]
        );
        assert_eq!(fit(lines.clone(), 3), vec![person("Minnow", false), person("Bream", true), and_more("Eel", 2)]);
        assert_eq!(fit(lines.clone(), 2), vec![person("Bream", true), and_more("Eel", 3)]);
        assert_eq!(fit(lines.clone(), 1), vec![and_more("Bream", 4)]);
        assert_eq!(fit(lines, 0), Vec::new());
        assert_eq!(fit(Vec::new(), 0), Vec::new());
    }

    #[test]
    fn headings_are_dropped_with_their_people() {
        let lines = vec![
            heading("Reef"),
            person("Minnow", false),
            person("Pike", false),
            heading("Harbour"),
            person("Eel", true),
        ];
        assert_eq!(fit(lines.clone(), 5), lines);
        assert_eq!(
            fit(lines.clone(), 4),
            vec![heading("Reef"), person("Minnow", false), heading("Harbour"), and_more("Eel", 1)]
        );
        assert_eq!(fit(lines.clone(), 3), vec![heading("Harbour"), and_more("Eel", 2)]);
        assert_eq!(fit(lines.clone(), 2), vec![heading("Harbour"), and_more("Eel", 2)]);
        assert_eq!(fit(lines.clone(), 1), vec![and_more("Eel", 2)], "with one line the name matters more than the server");
        assert_eq!(fit(vec![heading("Reef"), person("Pike", false)], 1), vec![person("Pike", false)]);
        assert_eq!(fit(lines, 0), Vec::new());
    }
}
