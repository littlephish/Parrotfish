pub struct Bits<'a> {
    data: &'a [u8],
    pos: u64,
    overflow: bool,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, overflow: false }
    }

    pub fn total(&self) -> u64 {
        self.data.len() as u64 * 8
    }

    pub fn pos(&self) -> u64 {
        self.pos
    }

    pub fn seek(&mut self, pos: u64) {
        self.pos = pos.min(self.total());
        self.overflow = false;
    }

    pub fn overflowed(&self) -> bool {
        self.overflow
    }

    pub fn remaining(&self) -> i64 {
        if self.overflow {
            -1
        } else {
            (self.total() - self.pos) as i64
        }
    }

    fn bit(&self, at: u64) -> u32 {
        let byte = self.data.get((at >> 3) as usize).copied().unwrap_or(0);
        (u32::from(byte) >> (7 - (at & 7) as u32)) & 1
    }

    pub fn unpack(&mut self, nb: u32) -> u32 {
        if self.pos + u64::from(nb) > self.total() {
            self.overflow = true;
        }
        if self.overflow {
            return 0;
        }
        let mut d = 0u32;
        for _ in 0..nb {
            d = (d << 1) | self.bit(self.pos);
            self.pos += 1;
        }
        d
    }

    pub fn peek(&mut self) -> u32 {
        if self.pos + 1 > self.total() {
            self.overflow = true;
        }
        if self.overflow {
            return 0;
        }
        self.bit(self.pos)
    }

    pub fn peek_at(&self, at: u64, nb: u32) -> Option<u32> {
        if at + u64::from(nb) > self.total() {
            return None;
        }
        let mut d = 0u32;
        for i in 0..u64::from(nb) {
            d = (d << 1) | self.bit(at + i);
        }
        Some(d)
    }

    pub fn advance(&mut self, n: i32) {
        let next = self.pos as i64 + i64::from(n);
        if self.overflow || next < 0 || next > self.total() as i64 {
            self.overflow = true;
            return;
        }
        self.pos = next as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_come_out_most_significant_first() {
        let mut bits = Bits::new(&[0b1011_0010, 0b0100_0001]);
        assert_eq!(bits.unpack(1), 1);
        assert_eq!(bits.unpack(4), 0b0110);
        assert_eq!(bits.unpack(3), 0b010);
        assert_eq!(bits.unpack(8), 0b0100_0001);
        assert_eq!(bits.remaining(), 0);
    }

    #[test]
    fn reading_past_the_end_gives_zeros_and_marks_overflow() {
        let mut bits = Bits::new(&[0xFF]);
        assert_eq!(bits.unpack(7), 0x7F);
        assert_eq!(bits.unpack(4), 0);
        assert!(bits.overflowed());
        assert_eq!(bits.remaining(), -1);
        assert_eq!(bits.unpack(1), 0);
    }

    #[test]
    fn an_empty_packet_has_nothing_to_read() {
        let mut bits = Bits::new(&[]);
        assert_eq!(bits.remaining(), 0);
        assert_eq!(bits.peek(), 0);
        assert!(bits.overflowed());
    }

    #[test]
    fn peeking_leaves_the_reader_where_it_was() {
        let mut bits = Bits::new(&[0b0100_0000, 0x00]);
        assert_eq!(bits.peek(), 0);
        assert_eq!(bits.pos(), 0);
        assert_eq!(bits.peek_at(1, 3), Some(0b100));
        assert_eq!(bits.peek_at(14, 4), None);
        assert_eq!(bits.pos(), 0);
    }

    #[test]
    fn advancing_moves_forwards_and_backwards() {
        let mut bits = Bits::new(&[0b0000_1111, 0b1010_0000]);
        bits.advance(4);
        assert_eq!(bits.unpack(4), 0b1111);
        bits.advance(-8);
        assert_eq!(bits.pos(), 0);
        bits.advance(17);
        assert!(bits.overflowed());
    }

    #[test]
    fn seeking_clears_the_overflow_mark() {
        let mut bits = Bits::new(&[0x81]);
        assert_eq!(bits.unpack(9), 0);
        assert!(bits.overflowed());
        bits.seek(0);
        assert!(!bits.overflowed());
        assert_eq!(bits.unpack(8), 0x81);
    }
}
