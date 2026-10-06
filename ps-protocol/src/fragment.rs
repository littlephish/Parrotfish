use std::collections::HashMap;

use crate::packet::{FLAG_COMPRESSED, FLAG_FRAGMENTED};
use crate::window::ReceiveWindow;
use crate::ProtocolError;

pub const MAX_DECOMPRESSED_SIZE: u32 = 2 * 1024 * 1024;
pub const MAX_FRAGMENTED_SIZE: usize = 1024 * 1024;
pub const MAX_AHEAD: u16 = 1024;

pub fn compress(data: &[u8]) -> Vec<u8> {
    quicklz::compress(data, quicklz::CompressionLevel::Lvl1)
}

pub fn decompress(data: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    let owned = data.to_vec();
    let result = std::panic::catch_unwind(move || {
        quicklz::decompress(&mut std::io::Cursor::new(&owned[..]), MAX_DECOMPRESSED_SIZE)
    });
    match result {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(ProtocolError::Decompress(e.to_string())),
        Err(_) => Err(ProtocolError::Decompress("malformed compressed data".into())),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutFragment {
    pub flags: u8,
    pub data: Vec<u8>,
}

pub fn split_command(data: &[u8], max_payload: usize) -> Vec<OutFragment> {
    if data.len() <= max_payload {
        return vec![OutFragment { flags: 0, data: data.to_vec() }];
    }
    let compressed = compress(data);
    let (body, base_flags) = if compressed.len() < data.len() {
        (compressed, FLAG_COMPRESSED)
    } else {
        (data.to_vec(), 0)
    };
    if body.len() <= max_payload {
        return vec![OutFragment { flags: base_flags, data: body }];
    }
    let chunks: Vec<&[u8]> = body.chunks(max_payload).collect();
    let last = chunks.len() - 1;
    chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            let mut flags = 0;
            if i == 0 {
                flags |= base_flags | FLAG_FRAGMENTED;
            }
            if i == last {
                flags |= FLAG_FRAGMENTED;
            }
            OutFragment { flags, data: chunk.to_vec() }
        })
        .collect()
}

#[derive(Debug, Default)]
pub struct Reassembler {
    partial: Option<(bool, Vec<u8>)>,
}

impl Reassembler {
    pub fn push(&mut self, flags: u8, payload: &[u8]) -> Result<Option<Vec<u8>>, ProtocolError> {
        let fragmented = flags & FLAG_FRAGMENTED != 0;
        match self.partial.take() {
            None => {
                let compressed = flags & FLAG_COMPRESSED != 0;
                if fragmented {
                    self.partial = Some((compressed, payload.to_vec()));
                    Ok(None)
                } else if compressed {
                    decompress(payload).map(Some)
                } else {
                    Ok(Some(payload.to_vec()))
                }
            }
            Some((compressed, mut buf)) => {
                if buf.len() + payload.len() > MAX_FRAGMENTED_SIZE {
                    return Err(ProtocolError::TooLarge("fragmented command"));
                }
                buf.extend_from_slice(payload);
                if !fragmented {
                    self.partial = Some((compressed, buf));
                    Ok(None)
                } else if compressed {
                    decompress(&buf).map(Some)
                } else {
                    Ok(Some(buf))
                }
            }
        }
    }

    pub fn in_progress(&self) -> bool {
        self.partial.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Next,
    Ahead,
    Old,
    TooFar,
}

#[derive(Debug, Default)]
pub struct CommandQueue {
    pub window: ReceiveWindow,
    pending: HashMap<u16, (u8, Vec<u8>)>,
    reassembler: Reassembler,
}

impl CommandQueue {
    pub fn classify(&self, id: u16) -> Slot {
        let distance = self.window.distance(id);
        if distance == 0 {
            Slot::Next
        } else if distance < MAX_AHEAD {
            Slot::Ahead
        } else if distance >= ReceiveWindow::SIZE {
            Slot::Old
        } else {
            Slot::TooFar
        }
    }

    pub fn has_pending(&self, id: u16) -> bool {
        self.pending.contains_key(&id)
    }

    pub fn insert(
        &mut self,
        id: u16,
        flags: u8,
        payload: Vec<u8>,
    ) -> Vec<Result<Vec<u8>, ProtocolError>> {
        let mut out = Vec::new();
        match self.classify(id) {
            Slot::Next => {
                self.deliver(flags, &payload, &mut out);
                while let Some((f, p)) = self.pending.remove(&self.window.next_id) {
                    self.deliver(f, &p, &mut out);
                }
            }
            Slot::Ahead => {
                self.pending.entry(id).or_insert((flags, payload));
            }
            Slot::Old | Slot::TooFar => {}
        }
        out
    }

    fn deliver(&mut self, flags: u8, payload: &[u8], out: &mut Vec<Result<Vec<u8>, ProtocolError>>) {
        self.window.bump();
        match self.reassembler.push(flags, payload) {
            Ok(Some(command)) => out.push(Ok(command)),
            Ok(None) => {}
            Err(e) => out.push(Err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::MAX_C2S_PAYLOAD;

    fn sample(len: usize, entropy: bool) -> Vec<u8> {
        let mut state = 0x1234_5678u32;
        (0..len)
            .map(|i| {
                if entropy {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    (state >> 8) as u8
                } else {
                    b"channel_name=Lobby\\s"[i % 20]
                }
            })
            .collect()
    }

    fn reassemble(fragments: &[OutFragment]) -> Vec<u8> {
        let mut r = Reassembler::default();
        let mut done = None;
        for (i, f) in fragments.iter().enumerate() {
            let res = r.push(f.flags, &f.data).unwrap();
            if i + 1 < fragments.len() {
                assert!(res.is_none());
                assert!(r.in_progress());
            } else {
                done = res;
            }
        }
        assert!(!r.in_progress());
        done.unwrap()
    }

    #[test]
    fn small_commands_are_untouched() {
        let data = sample(MAX_C2S_PAYLOAD, false);
        let f = split_command(&data, MAX_C2S_PAYLOAD);
        assert_eq!(f, vec![OutFragment { flags: 0, data: data.clone() }]);
        assert_eq!(reassemble(&f), data);
    }

    #[test]
    fn compressible_command_fits_after_compression() {
        let data = sample(3000, false);
        let f = split_command(&data, MAX_C2S_PAYLOAD);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].flags, FLAG_COMPRESSED);
        assert!(f[0].data.len() < data.len());
        assert_eq!(reassemble(&f), data);
    }

    #[test]
    fn incompressible_command_is_split() {
        let data = sample(MAX_C2S_PAYLOAD * 3 + 17, true);
        let f = split_command(&data, MAX_C2S_PAYLOAD);
        assert!(f.len() >= 4);
        assert!(f.iter().all(|x| x.data.len() <= MAX_C2S_PAYLOAD));
        assert_ne!(f[0].flags & FLAG_FRAGMENTED, 0);
        assert_ne!(f[f.len() - 1].flags & FLAG_FRAGMENTED, 0);
        for mid in &f[1..f.len() - 1] {
            assert_eq!(mid.flags, 0);
        }
        assert_eq!(f[f.len() - 1].flags & FLAG_COMPRESSED, 0);
        assert_eq!(reassemble(&f), data);
    }

    #[test]
    fn large_compressible_command_is_compressed_then_split() {
        let mut data = sample(40_000, false);
        let noise = sample(4_000, true);
        for (i, b) in noise.iter().enumerate() {
            data[i * 10] = *b;
        }
        let f = split_command(&data, MAX_C2S_PAYLOAD);
        assert!(f.len() > 1);
        assert_eq!(f[0].flags, FLAG_COMPRESSED | FLAG_FRAGMENTED);
        assert_eq!(f[f.len() - 1].flags, FLAG_FRAGMENTED);
        let total: usize = f.iter().map(|x| x.data.len()).sum();
        assert!(total < data.len());
        assert_eq!(reassemble(&f), data);
    }

    #[test]
    fn decompress_rejects_garbage() {
        assert!(decompress(&[]).is_err());
        assert!(decompress(&[0x47, 0xff, 0xff, 0xff, 0x7f, 0xff, 0xff, 0xff, 0x7f]).is_err());
        let mut r = Reassembler::default();
        assert!(r.push(FLAG_COMPRESSED, &[1, 2, 3]).is_err());
        assert_eq!(r.push(0, b"ok").unwrap().unwrap(), b"ok");
    }

    #[test]
    fn queue_delivers_in_order() {
        let mut q = CommandQueue::default();
        assert_eq!(q.classify(0), Slot::Next);
        assert_eq!(q.classify(5), Slot::Ahead);
        assert_eq!(q.classify(65535), Slot::Old);
        assert_eq!(q.classify(MAX_AHEAD), Slot::TooFar);

        assert!(q.insert(2, 0, b"c".to_vec()).is_empty());
        assert!(q.insert(1, 0, b"b".to_vec()).is_empty());
        assert!(q.has_pending(2));
        assert!(q.insert(2, 0, b"duplicate".to_vec()).is_empty());
        let out = q.insert(0, 0, b"a".to_vec());
        let texts: Vec<Vec<u8>> = out.into_iter().map(|r| r.unwrap()).collect();
        assert_eq!(texts, vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
        assert_eq!(q.window.next_id, 3);
        assert_eq!(q.classify(1), Slot::Old);
        assert!(q.insert(1, 0, b"late".to_vec()).is_empty());
        assert_eq!(q.window.next_id, 3);
    }

    #[test]
    fn queue_reassembles_out_of_order_fragments() {
        let data = sample(MAX_C2S_PAYLOAD * 2 + 100, true);
        let f = split_command(&data, MAX_C2S_PAYLOAD);
        assert_eq!(f.len(), 3);
        let mut q = CommandQueue::default();
        assert!(q.insert(2, f[2].flags, f[2].data.clone()).is_empty());
        assert!(q.insert(0, f[0].flags, f[0].data.clone()).is_empty());
        let out = q.insert(1, f[1].flags, f[1].data.clone());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].as_ref().unwrap(), &data);
        let out = q.insert(3, 0, b"next".to_vec());
        assert_eq!(out[0].as_ref().unwrap(), b"next");
    }

    #[test]
    fn queue_survives_id_wrap() {
        let mut q = CommandQueue::default();
        q.window = ReceiveWindow { next_id: 65535, generation: 0 };
        assert!(q.insert(0, 0, b"second".to_vec()).is_empty());
        let out = q.insert(65535, 0, b"first".to_vec());
        assert_eq!(out.len(), 2);
        assert_eq!(q.window, ReceiveWindow { next_id: 1, generation: 1 });
    }
}
