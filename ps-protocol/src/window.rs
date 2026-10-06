#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PacketCounter {
    pub id: u16,
    pub generation: u32,
}

impl PacketCounter {
    pub fn starting_at(id: u16) -> Self {
        Self { id, generation: 0 }
    }

    pub fn next(&mut self) -> (u16, u32) {
        let current = (self.id, self.generation);
        self.id = self.id.wrapping_add(1);
        if self.id == 0 {
            self.generation = self.generation.wrapping_add(1);
        }
        current
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReceiveWindow {
    pub next_id: u16,
    pub generation: u32,
}

impl ReceiveWindow {
    pub const SIZE: u16 = u16::MAX / 2;

    pub fn locate(&self, id: u16) -> (bool, u32) {
        let cur = self.next_id;
        let (limit, wraps) = cur.overflowing_add(Self::SIZE);
        let in_window = if wraps { id >= cur || id < limit } else { id >= cur && id < limit };
        let generation = if in_window {
            if wraps && id < limit {
                self.generation.wrapping_add(1)
            } else {
                self.generation
            }
        } else if id < cur {
            self.generation
        } else {
            self.generation.saturating_sub(1)
        };
        (in_window, generation)
    }

    pub fn distance(&self, id: u16) -> u16 {
        id.wrapping_sub(self.next_id)
    }

    pub fn advance_past(&mut self, id: u16, generation: u32) {
        let key = ((generation as u64) << 16) | id as u64;
        let cur = ((self.generation as u64) << 16) | self.next_id as u64;
        if key >= cur {
            let next = key + 1;
            self.next_id = next as u16;
            self.generation = (next >> 16) as u32;
        }
    }

    pub fn bump(&mut self) {
        self.advance_past(self.next_id, self.generation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_wraps_into_next_generation() {
        let mut c = PacketCounter::starting_at(65534);
        assert_eq!(c.next(), (65534, 0));
        assert_eq!(c.next(), (65535, 0));
        assert_eq!(c.next(), (0, 1));
        assert_eq!(c.next(), (1, 1));
        let mut d = PacketCounter::default();
        assert_eq!(d.next(), (0, 0));
        assert_eq!(d.next(), (1, 0));
    }

    #[test]
    fn locate_without_wrap() {
        let w = ReceiveWindow { next_id: 100, generation: 3 };
        assert_eq!(w.locate(100), (true, 3));
        assert_eq!(w.locate(5000), (true, 3));
        assert_eq!(w.locate(100 + ReceiveWindow::SIZE - 1), (true, 3));
        assert_eq!(w.locate(100 + ReceiveWindow::SIZE), (false, 2));
        assert_eq!(w.locate(99), (false, 3));
        assert_eq!(w.locate(65535), (false, 2));
        assert_eq!(w.locate(0), (false, 3));
    }

    #[test]
    fn locate_across_wrap() {
        let w = ReceiveWindow { next_id: 65000, generation: 1 };
        assert_eq!(w.locate(65000), (true, 1));
        assert_eq!(w.locate(65535), (true, 1));
        assert_eq!(w.locate(0), (true, 2));
        assert_eq!(w.locate(100), (true, 2));
        assert_eq!(w.locate(64999), (false, 1));
        assert_eq!(w.locate(40000), (false, 1));
    }

    #[test]
    fn first_generation_never_underflows() {
        let w = ReceiveWindow::default();
        assert_eq!(w.locate(65535), (false, 0));
        assert_eq!(w.locate(0), (true, 0));
    }

    #[test]
    fn advance_only_moves_forward() {
        let mut w = ReceiveWindow::default();
        w.advance_past(10, 0);
        assert_eq!(w, ReceiveWindow { next_id: 11, generation: 0 });
        w.advance_past(5, 0);
        assert_eq!(w, ReceiveWindow { next_id: 11, generation: 0 });
        w.advance_past(65535, 0);
        assert_eq!(w, ReceiveWindow { next_id: 0, generation: 1 });
        w.advance_past(65535, 0);
        assert_eq!(w, ReceiveWindow { next_id: 0, generation: 1 });
        w.bump();
        assert_eq!(w, ReceiveWindow { next_id: 1, generation: 1 });
        assert_eq!(w.distance(4), 3);
        assert_eq!(w.distance(0), 65535);
    }

    #[test]
    fn sender_and_receiver_agree_over_many_generations() {
        let mut tx = PacketCounter::default();
        let mut rx = ReceiveWindow::default();
        for i in 0..200_000u32 {
            let (id, generation) = tx.next();
            if i % 7 == 3 {
                continue;
            }
            let (in_window, located) = rx.locate(id);
            assert!(in_window);
            assert_eq!(located, generation, "packet {i}");
            rx.advance_past(id, located);
        }
        assert_eq!(rx.generation, 3);
    }
}
