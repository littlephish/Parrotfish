use std::time::{Duration, Instant};

pub const SETTLE: Duration = Duration::from_secs(2);
pub const OFF_SPACING: Duration = Duration::from_secs(15);
pub const ANSWER_WAIT: Duration = Duration::from_secs(3);
pub const LONGEST_WAIT: Duration = Duration::from_secs(30);
pub const REFUSAL_MARGIN: Duration = Duration::from_millis(250);
pub const REFUSAL_WAIT: Duration = Duration::from_secs(6);
pub const LONGEST_REFUSAL_WAIT: Duration = Duration::from_secs(120);

pub const MARK_NONE: i32 = 0;
pub const MARK_HERE: i32 = 1;
pub const MARK_OFF: i32 = 2;

pub fn mark(connected: bool, holds: bool, connected_servers: usize) -> i32 {
    if !connected || connected_servers < 2 {
        MARK_NONE
    } else if holds {
        MARK_HERE
    } else {
        MARK_OFF
    }
}

pub fn refusal_wait(extra: &str) -> Duration {
    let named = extra.split("ms").next().and_then(|before| {
        let digits: String = before.trim_end().chars().rev().take_while(char::is_ascii_digit).collect();
        digits.chars().rev().collect::<String>().parse::<u64>().ok()
    });
    named.map_or(REFUSAL_WAIT, Duration::from_millis).min(LONGEST_REFUSAL_WAIT) + REFUSAL_MARGIN
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    told: bool,
    asked: Option<Instant>,
    tries: u32,
    away: Option<Instant>,
    last_off: Option<Instant>,
    quiet_until: Option<Instant>,
    doubted: bool,
}

impl Report {
    pub fn new(told: bool) -> Self {
        Self { told, asked: None, tries: 0, away: None, last_off: None, quiet_until: None, doubted: false }
    }

    fn answer_wait(&self) -> Duration {
        let doublings = self.tries.saturating_sub(1).min(8);
        (ANSWER_WAIT * (1u32 << doublings)).min(LONGEST_WAIT)
    }

    fn waiting(&self, now: Instant) -> bool {
        self.asked.is_some_and(|at| now.saturating_duration_since(at) < self.answer_wait())
    }

    fn believed(&self, server_says: Option<bool>, now: Instant) -> bool {
        if self.waiting(now) {
            self.told
        } else {
            server_says.unwrap_or(self.told)
        }
    }

    pub fn seems_on(&self, server_says: Option<bool>) -> bool {
        if self.doubted {
            server_says.unwrap_or(self.told)
        } else {
            self.told
        }
    }

    pub fn refused(&mut self, until: Instant) {
        self.quiet_until = Some(self.quiet_until.map_or(until, |known| known.max(until)));
        self.asked = None;
        self.doubted = true;
    }

    pub fn step(&mut self, here: bool, server_says: Option<bool>, now: Instant) -> Option<bool> {
        if here {
            self.away = None;
        } else if self.away.is_none() {
            self.away = Some(now);
        }
        if server_says == Some(self.told) {
            self.doubted = false;
        }
        if self.believed(server_says, now) == here {
            if !self.waiting(now) {
                self.tries = 0;
            }
            return None;
        }
        if self.quiet_until.is_some_and(|until| now < until) {
            return None;
        }
        if !here {
            let settled = self.away.is_some_and(|since| now.saturating_duration_since(since) >= SETTLE);
            let spaced = self.last_off.map_or(true, |at| now.saturating_duration_since(at) >= OFF_SPACING);
            if !settled || !spaced {
                return None;
            }
            self.last_off = Some(now);
        }
        self.tries = if self.told == here { self.tries + 1 } else { 1 };
        self.told = here;
        self.asked = Some(now);
        Some(here)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(start: Instant, millis: u64) -> Instant {
        start + Duration::from_millis(millis)
    }

    fn run(report: &mut Report, start: Instant, from: u64, to: u64, here: bool, server_says: Option<bool>) -> Vec<(u64, bool)> {
        let mut sent = Vec::new();
        let mut at = from;
        while at <= to {
            if let Some(on) = report.step(here, server_says, ms(start, at)) {
                sent.push((at, on));
            }
            at += 50;
        }
        sent
    }

    struct Server {
        points: f64,
        at: u64,
        on: bool,
    }

    impl Server {
        const COST: f64 = 15.0;
        const LIMIT: f64 = 150.0;
        const DRAIN_PER_SECOND: f64 = 5.0;

        fn new(points: f64, on: bool) -> Self {
            Self { points, at: 0, on }
        }

        fn drain(&mut self, now: u64) {
            self.points = (self.points - Self::DRAIN_PER_SECOND * (now - self.at) as f64 / 1000.0).max(0.0);
            self.at = now;
        }

        fn command(&mut self, now: u64, on: bool) -> Result<(), u64> {
            self.drain(now);
            self.points += Self::COST;
            if self.points > Self::LIMIT {
                Err(((self.points - (Self::LIMIT - Self::COST)) / Self::DRAIN_PER_SECOND * 1000.0) as u64)
            } else {
                self.on = on;
                Ok(())
            }
        }
    }

    #[test]
    fn one_server_is_never_told_anything() {
        let start = Instant::now();
        let mut report = Report::new(true);
        assert_eq!(run(&mut report, start, 0, 60_000, true, Some(true)), vec![]);
        assert!(report.seems_on(Some(true)));
    }

    #[test]
    fn a_server_that_loses_the_microphone_is_told_after_the_settle_time() {
        let start = Instant::now();
        let mut report = Report::new(true);
        assert_eq!(run(&mut report, start, 0, 1_950, false, Some(true)), vec![]);
        assert_eq!(report.step(false, Some(true), ms(start, 2_000)), Some(false));
        assert_eq!(run(&mut report, start, 2_050, 60_000, false, Some(false)), vec![]);
        assert!(!report.seems_on(Some(false)));
    }

    #[test]
    fn a_short_look_at_another_server_costs_nothing() {
        let start = Instant::now();
        let mut report = Report::new(true);
        assert_eq!(run(&mut report, start, 0, 1_900, false, Some(true)), vec![]);
        assert_eq!(run(&mut report, start, 1_950, 10_000, true, Some(true)), vec![]);
        assert_eq!(run(&mut report, start, 10_050, 11_900, false, Some(true)), vec![]);
        assert_eq!(report.step(false, Some(true), ms(start, 12_050)), Some(false));
    }

    #[test]
    fn the_server_that_gets_the_microphone_is_told_at_once() {
        let start = Instant::now();
        let mut report = Report::new(false);
        assert_eq!(report.step(false, Some(false), start), None);
        assert_eq!(report.step(true, Some(false), ms(start, 5_000)), Some(true));
        assert!(report.seems_on(Some(false)), "no waiting for the server's answer before it shows");
        assert_eq!(run(&mut report, start, 5_050, 30_000, true, Some(true)), vec![]);
    }

    #[test]
    fn coming_back_right_after_off_was_sent_turns_it_on_again_at_once() {
        let start = Instant::now();
        let mut report = Report::new(true);
        assert_eq!(report.step(false, Some(true), start), None);
        assert_eq!(report.step(false, Some(true), ms(start, 2_000)), Some(false));
        assert_eq!(
            report.step(true, Some(true), ms(start, 2_010)),
            Some(true),
            "the server's old answer is not taken for the new state"
        );
        assert_eq!(run(&mut report, start, 2_050, 4_900, true, Some(true)), vec![]);
    }

    #[test]
    fn switching_back_and_forth_is_held_to_one_off_every_spacing() {
        let start = Instant::now();
        let mut report = Report::new(true);
        let mut sent = Vec::new();
        let mut says = true;
        let mut at = 0u64;
        while at < 120_000 {
            let here = (at / 2_500) % 2 == 1;
            if let Some(on) = report.step(here, Some(says), ms(start, at)) {
                sent.push((at, on));
                says = on;
            }
            at += 50;
        }
        let offs: Vec<u64> = sent.iter().filter(|(_, on)| !on).map(|(at, _)| *at).collect();
        assert!(offs.len() >= 4, "{offs:?}");
        assert!(offs.windows(2).all(|pair| pair[1] - pair[0] >= OFF_SPACING.as_millis() as u64), "{offs:?}");
        assert!(sent.len() <= 2 * (120_000 / OFF_SPACING.as_millis() as usize) + 2, "{} reports in two minutes", sent.len());
        for pair in sent.windows(2) {
            assert_ne!(pair[0].1, pair[1].1, "{sent:?}");
        }
    }

    #[test]
    fn switching_back_and_forth_for_ten_minutes_is_never_refused_by_a_default_server() {
        for period in [700u64, 2_050, 2_500, 5_000, 7_600, 17_000] {
            let start = Instant::now();
            let mut report = Report::new(true);
            let mut server = Server::new(0.0, true);
            let mut at = 0u64;
            while at < 600_000 {
                let here = (at / period) % 2 == 1;
                if let Some(on) = report.step(here, Some(server.on), ms(start, at)) {
                    assert_eq!(server.command(at, on), Ok(()), "switching every {period} ms, {at} ms in");
                    assert!(server.points <= Server::LIMIT / 3.0, "{} points, switching every {period} ms", server.points);
                }
                if here {
                    assert!(
                        report.seems_on(Some(server.on)),
                        "switching every {period} ms, {at} ms in: the server with the microphone must count as on"
                    );
                }
                at += 50;
            }
        }
    }

    #[test]
    fn an_unanswered_on_is_asked_again_less_and_less_often() {
        let start = Instant::now();
        let mut report = Report::new(false);
        let sent = run(&mut report, start, 0, 110_000, true, Some(false));
        let times: Vec<u64> = sent.iter().map(|(at, _)| *at).collect();
        assert_eq!(times, vec![0, 3_000, 9_000, 21_000, 45_000, 75_000, 105_000]);
        assert!(sent.iter().all(|(_, on)| *on));
    }

    #[test]
    fn what_is_shown_is_what_was_sent_until_the_server_refuses_something() {
        let start = Instant::now();
        let mut report = Report::new(false);
        assert_eq!(report.step(true, Some(false), start), Some(true));
        assert!(report.seems_on(Some(false)));
        assert_eq!(report.step(true, Some(false), ms(start, 3_000)), Some(true));
        assert!(report.seems_on(Some(false)), "a server that never repeats our own state back is not taken for a refusal");
        report.refused(ms(start, 9_000));
        assert!(!report.seems_on(Some(false)), "after a refusal the server's word is shown");
        assert!(report.seems_on(None));
        assert_eq!(run(&mut report, start, 3_050, 8_950, true, Some(false)), vec![]);
        assert_eq!(report.step(true, Some(false), ms(start, 9_000)), Some(true));
        assert!(!report.seems_on(Some(false)), "sending again does not make it look on");
        assert_eq!(report.step(true, Some(true), ms(start, 9_100)), None);
        assert!(report.seems_on(Some(true)), "it shows on as soon as the server agrees");
        assert!(report.seems_on(Some(false)), "and then what was sent is believed again");
    }

    #[test]
    fn what_is_shown_follows_a_fresh_report_without_waiting_for_the_answer() {
        let start = Instant::now();
        let mut report = Report::new(true);
        assert_eq!(report.step(false, Some(true), start), None);
        assert!(report.seems_on(Some(true)), "nothing was sent yet, so it still counts as on");
        assert_eq!(report.step(false, Some(true), ms(start, 2_000)), Some(false));
        assert!(!report.seems_on(Some(true)));
        assert_eq!(report.step(true, Some(true), ms(start, 2_500)), Some(true));
        assert!(report.seems_on(Some(false)));
    }

    #[test]
    fn a_refused_on_waits_the_time_the_server_named_and_then_gets_through() {
        let start = Instant::now();
        let mut report = Report::new(false);
        let mut server = Server::new(140.0, false);
        let mut sent = Vec::new();
        let mut heard_at = None;
        let mut at = 0u64;
        while at < 30_000 {
            let now = ms(start, at);
            if let Some(on) = report.step(true, Some(server.on), now) {
                sent.push(at);
                if let Err(wait) = server.command(at, on) {
                    report.refused(now + refusal_wait(&format!("retry in {wait}ms")));
                    assert!(!report.seems_on(Some(server.on)), "a refused report is not shown as on");
                }
            }
            if server.on && heard_at.is_none() {
                heard_at = Some(at);
            }
            if heard_at.is_none() && at > 0 {
                assert!(!report.seems_on(Some(server.on)), "{at} ms in");
            }
            at += 50;
        }
        assert_eq!(sent.len(), 2, "{sent:?}");
        assert_eq!(sent[0], 0);
        assert!((4_250..=4_350).contains(&sent[1]), "{sent:?}");
        assert_eq!(heard_at, Some(sent[1]));
        assert!(report.seems_on(Some(true)));
    }

    #[test]
    fn a_refused_on_gets_through_even_if_the_refusal_is_never_seen() {
        let start = Instant::now();
        let mut report = Report::new(false);
        let mut server = Server::new(140.0, false);
        let mut sent = Vec::new();
        let mut at = 0u64;
        while at < 60_000 {
            if let Some(on) = report.step(true, Some(server.on), ms(start, at)) {
                sent.push(at);
                let _ = server.command(at, on);
            }
            at += 50;
        }
        assert_eq!(sent, vec![0, 3_000, 9_000]);
        assert!(server.on);
    }

    #[test]
    fn while_the_server_refuses_commands_nothing_is_sent_at_all() {
        let start = Instant::now();
        let mut report = Report::new(false);
        report.refused(ms(start, 5_000));
        assert_eq!(run(&mut report, start, 0, 4_950, true, Some(false)), vec![]);
        assert!(!report.seems_on(Some(false)));
        assert_eq!(report.step(true, Some(false), ms(start, 5_000)), Some(true));
        let mut leaving = Report::new(true);
        leaving.refused(ms(start, 9_000));
        assert_eq!(run(&mut leaving, start, 0, 8_950, false, Some(true)), vec![]);
        assert_eq!(leaving.step(false, Some(true), ms(start, 9_000)), Some(false));
        let mut later = Report::new(false);
        later.refused(ms(start, 9_000));
        later.refused(ms(start, 4_000));
        assert_eq!(run(&mut later, start, 0, 8_950, true, Some(false)), vec![], "an earlier time does not shorten the wait");
    }

    #[test]
    fn a_refusal_of_something_else_does_not_repeat_a_report_the_server_took() {
        let start = Instant::now();
        let mut report = Report::new(false);
        assert_eq!(report.step(true, Some(false), start), Some(true));
        report.refused(ms(start, 6_100));
        assert_eq!(run(&mut report, start, 50, 60_000, true, Some(true)), vec![]);
    }

    #[test]
    fn an_unanswered_off_is_asked_again_at_the_slow_pace() {
        let start = Instant::now();
        let mut report = Report::new(true);
        let sent = run(&mut report, start, 0, 40_000, false, Some(true));
        assert_eq!(sent, vec![(2_000, false), (17_000, false), (32_000, false)]);
    }

    #[test]
    fn a_refused_off_is_not_hurried() {
        let start = Instant::now();
        let mut report = Report::new(true);
        let mut server = Server::new(150.0, true);
        let mut sent = Vec::new();
        let mut refusals = 0;
        let mut at = 0u64;
        while at < 40_000 {
            let now = ms(start, at);
            let here = at >= 30_000;
            if let Some(on) = report.step(here, Some(server.on), now) {
                sent.push((at, on));
                if let Err(wait) = server.command(at, on) {
                    refusals += 1;
                    report.refused(now + refusal_wait(&format!("retry in {wait}ms")));
                }
            }
            at += 50;
        }
        assert_eq!(refusals, 1);
        assert_eq!(
            sent,
            vec![(2_000, false), (17_000, false), (30_000, true)],
            "the off that was refused is sent again when the spacing allows, not when the server would first take it"
        );
        assert!(server.on);
        assert!(report.seems_on(Some(server.on)));
    }

    #[test]
    fn coming_back_after_a_refused_off_needs_no_report() {
        let start = Instant::now();
        let mut report = Report::new(true);
        assert_eq!(report.step(false, Some(true), start), None);
        assert_eq!(report.step(false, Some(true), ms(start, 2_000)), Some(false));
        report.refused(ms(start, 6_250));
        assert_eq!(run(&mut report, start, 2_050, 60_000, true, Some(true)), vec![]);
        assert!(report.seems_on(Some(true)));
    }

    #[test]
    fn without_word_from_the_server_what_was_sent_is_believed() {
        let start = Instant::now();
        let mut report = Report::new(true);
        assert_eq!(run(&mut report, start, 0, 30_000, false, None), vec![(2_000, false)]);
        assert!(!report.seems_on(None));
        assert_eq!(run(&mut report, start, 30_050, 60_000, true, None), vec![(30_050, true)]);
        assert!(report.seems_on(None));
    }

    #[test]
    fn a_connection_that_signed_in_with_the_microphone_off_and_has_it_is_corrected_at_once() {
        let start = Instant::now();
        let mut report = Report::new(false);
        assert_eq!(report.step(true, None, start), Some(true));
        let mut signed_in_on = Report::new(true);
        assert_eq!(run(&mut signed_in_on, start, 0, 1_950, false, None), vec![]);
        assert_eq!(signed_in_on.step(false, None, ms(start, 2_000)), Some(false));
    }

    #[test]
    fn the_wait_a_server_names_is_read_from_its_answer() {
        assert_eq!(refusal_wait("retry in 5999ms"), Duration::from_millis(5_999) + REFUSAL_MARGIN);
        assert_eq!(refusal_wait("retry in 12ms"), Duration::from_millis(12) + REFUSAL_MARGIN);
        assert_eq!(refusal_wait("retry in 4500 ms"), Duration::from_millis(4_500) + REFUSAL_MARGIN);
        assert_eq!(refusal_wait(""), REFUSAL_WAIT + REFUSAL_MARGIN);
        assert_eq!(refusal_wait("please slow down"), REFUSAL_WAIT + REFUSAL_MARGIN);
        assert_eq!(refusal_wait("retry in ms"), REFUSAL_WAIT + REFUSAL_MARGIN);
        assert_eq!(refusal_wait("retry in 99999999999ms"), LONGEST_REFUSAL_WAIT + REFUSAL_MARGIN);
        assert_eq!(refusal_wait("retry in 99999999999999999999999999ms"), REFUSAL_WAIT + REFUSAL_MARGIN);
    }

    #[test]
    fn marks_only_appear_with_two_connected_servers() {
        assert_eq!(mark(true, true, 1), MARK_NONE);
        assert_eq!(mark(true, true, 2), MARK_HERE);
        assert_eq!(mark(true, false, 2), MARK_OFF);
        assert_eq!(mark(false, false, 3), MARK_NONE);
        assert_eq!(mark(true, false, 3), MARK_OFF);
    }
}
