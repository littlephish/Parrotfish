use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use rand::RngCore;

use crate::platform;

const FILE: &str = "instance";
const MAX_MESSAGE: usize = 8192;
const CONNECT_WAIT: Duration = Duration::from_millis(400);
const ANSWER_WAIT: Duration = Duration::from_secs(2);
const ACCEPTED: &[u8] = b"taken\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wish {
    Show,
    Link(String),
    Connect { target: String, nickname: String, channel: String },
}

fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

pub fn write_wishes(wishes: &[Wish]) -> String {
    let mut out = String::new();
    for wish in wishes {
        match wish {
            Wish::Show => out.push_str("show"),
            Wish::Link(link) => {
                out.push_str("link\t");
                out.push_str(&clean(link));
            }
            Wish::Connect { target, nickname, channel } => {
                out.push_str(&format!("connect\t{}\t{}\t{}", clean(target), clean(nickname), clean(channel)));
            }
        }
        out.push('\n');
    }
    out
}

pub fn read_wishes(text: &str) -> Vec<Wish> {
    let mut wishes = Vec::new();
    for line in text.lines() {
        let mut parts = line.split('\t');
        match (parts.next(), parts.next(), parts.next(), parts.next(), parts.next()) {
            (Some("show"), None, ..) => wishes.push(Wish::Show),
            (Some("link"), Some(link), None, ..) if !link.is_empty() => wishes.push(Wish::Link(link.to_string())),
            (Some("connect"), Some(target), Some(nickname), Some(channel), None) if !target.is_empty() => {
                wishes.push(Wish::Connect {
                    target: target.to_string(),
                    nickname: nickname.to_string(),
                    channel: channel.to_string(),
                });
            }
            _ => {}
        }
    }
    wishes
}

struct Card {
    port: u16,
    word: String,
    process: u32,
}

fn read_card(path: &Path) -> Option<Card> {
    let text = fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let port = lines.next()?.trim().parse().ok().filter(|port| *port != 0)?;
    let word = lines.next()?.trim().to_string();
    let process = lines.next()?.trim().parse().ok()?;
    (word.len() == 64 && word.bytes().all(|b| b.is_ascii_hexdigit())).then_some(Card { port, word, process })
}

fn same_word(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |sum, (x, y)| sum | (x ^ y)) == 0
}

pub fn hand_over(folder: &Path, wishes: &[Wish]) -> bool {
    let Some(card) = read_card(&folder.join(FILE)) else {
        return false;
    };
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, card.port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, CONNECT_WAIT) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(ANSWER_WAIT));
    let _ = stream.set_write_timeout(Some(ANSWER_WAIT));
    platform::allow_front(card.process);
    let message = format!("{}\n{}\n", card.word, write_wishes(wishes));
    if message.len() > MAX_MESSAGE || stream.write_all(message.as_bytes()).is_err() {
        return false;
    }
    let mut answer = [0u8; 6];
    stream.read_exact(&mut answer).is_ok() && answer == *ACCEPTED
}

pub struct Listener {
    pub wishes: Receiver<Wish>,
    path: PathBuf,
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn take(mut stream: TcpStream, word: &str, out: &Sender<Wish>) {
    let _ = stream.set_read_timeout(Some(ANSWER_WAIT));
    let _ = stream.set_write_timeout(Some(ANSWER_WAIT));
    let mut got = Vec::new();
    let mut piece = [0u8; 1024];
    while !got.ends_with(b"\n\n") && got.len() <= MAX_MESSAGE {
        match stream.read(&mut piece) {
            Ok(0) | Err(_) => return,
            Ok(n) => got.extend_from_slice(&piece[..n]),
        }
    }
    if got.len() > MAX_MESSAGE {
        return;
    }
    let Ok(text) = String::from_utf8(got) else {
        return;
    };
    let Some((first, rest)) = text.split_once('\n') else {
        return;
    };
    if !same_word(first.as_bytes(), word.as_bytes()) {
        return;
    }
    let wishes = read_wishes(rest);
    if stream.write_all(ACCEPTED).is_err() {
        return;
    }
    for wish in wishes {
        let _ = out.send(wish);
    }
}

pub fn listen(folder: &Path) -> Option<Listener> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    let mut random = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut random);
    let word: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    let path = folder.join(FILE);
    let draft = folder.join(format!("{FILE}.{}", std::process::id()));
    fs::create_dir_all(folder).ok()?;
    fs::write(&draft, format!("{port}\n{word}\n{}\n", std::process::id())).ok()?;
    if fs::rename(&draft, &path).is_err() {
        let _ = fs::remove_file(&draft);
        return None;
    }
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("ps-instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                take(stream, &word, &tx);
            }
        })
        .ok()?;
    Some(Listener { wishes: rx, path })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("phishspeak-instance-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn sample() -> Vec<Wish> {
        vec![
            Wish::Link("ts3server://example.org?channel=Lobby%2FSide".to_string()),
            Wish::Connect { target: "Reef Runners".to_string(), nickname: String::new(), channel: "Deep Rock".to_string() },
            Wish::Show,
        ]
    }

    #[test]
    fn wishes_survive_being_written_down() {
        assert_eq!(read_wishes(&write_wishes(&sample())), sample());
        assert_eq!(write_wishes(&[Wish::Show]), "show\n");
        assert_eq!(read_wishes(""), Vec::new());
        assert_eq!(read_wishes("show\tmore\nlink\t\nconnect\tonly\nnonsense\n\nlink\ta\tb\n"), Vec::new());
        let odd = Wish::Connect { target: "a\tb\nc".to_string(), nickname: "n\r".to_string(), channel: String::new() };
        assert_eq!(
            read_wishes(&write_wishes(&[odd])),
            vec![Wish::Connect { target: "abc".to_string(), nickname: "n".to_string(), channel: String::new() }],
            "line breaks and tabs cannot smuggle in a second wish"
        );
    }

    #[test]
    fn a_second_start_hands_its_wishes_to_the_first() {
        let folder = scratch("hand-over");
        assert!(!hand_over(&folder, &sample()), "nobody is there yet");
        let first = listen(&folder).expect("the first one listens");
        assert!(hand_over(&folder, &sample()));
        let got: Vec<Wish> = (0..3).map(|_| first.wishes.recv_timeout(Duration::from_secs(2)).unwrap()).collect();
        assert_eq!(got, sample());
        assert!(hand_over(&folder, &[Wish::Show]));
        assert_eq!(first.wishes.recv_timeout(Duration::from_secs(2)), Ok(Wish::Show));
        drop(first);
        assert!(!folder.join(FILE).exists(), "the note is taken down when the first one leaves");
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_note_left_behind_by_a_program_that_is_gone_is_ignored() {
        let folder = scratch("stale");
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
        fs::write(folder.join(FILE), format!("{port}\n{}\n1\n", "ab".repeat(32))).unwrap();
        assert!(!hand_over(&folder, &[Wish::Show]));
        for broken in ["", "notaport\nabc\n1\n", "80\nshort\n1\n", "0\n\n\n", &format!("70000\n{}\n1\n", "ab".repeat(32))] {
            fs::write(folder.join(FILE), broken).unwrap();
            assert!(!hand_over(&folder, &[Wish::Show]), "{broken:?}");
        }
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn only_someone_who_can_read_the_note_is_listened_to() {
        let folder = scratch("word");
        let first = listen(&folder).expect("the first one listens");
        let card = read_card(&folder.join(FILE)).unwrap();
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, card.port));
        for opening in [
            format!("{}\nlink\tts3server://evil.example\n\n", "0".repeat(64)),
            "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n\n\n".to_string(),
            "\n\n".to_string(),
            format!("{}\n", "x".repeat(MAX_MESSAGE + 100)),
        ] {
            let mut stream = TcpStream::connect(address).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let _ = stream.write_all(opening.as_bytes());
            let mut answer = Vec::new();
            let _ = stream.read_to_end(&mut answer);
            assert!(answer.is_empty(), "an answer to {:?}", &opening[..opening.len().min(30)]);
        }
        assert!(first.wishes.try_recv().is_err(), "nothing got through");
        assert!(hand_over(&folder, &[Wish::Link("ts3server://good.example".to_string())]), "and it still works afterwards");
        assert_eq!(
            first.wishes.recv_timeout(Duration::from_secs(2)),
            Ok(Wish::Link("ts3server://good.example".to_string()))
        );
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn something_else_listening_there_is_not_mistaken_for_phishspeak() {
        let folder = scratch("stranger");
        let stranger = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = stranger.local_addr().unwrap().port();
        fs::write(folder.join(FILE), format!("{port}\n{}\n1\n", "cd".repeat(32))).unwrap();
        let reply = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = stranger.accept() {
                let _ = stream.write_all(b"hello!");
            }
        });
        assert!(!hand_over(&folder, &[Wish::Show]));
        let _ = reply.join();
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn too_much_to_hand_over_is_not_sent() {
        let folder = scratch("big");
        let first = listen(&folder).expect("the first one listens");
        assert!(!hand_over(&folder, &[Wish::Link("x".repeat(MAX_MESSAGE))]));
        assert!(first.wishes.try_recv().is_err());
        let _ = fs::remove_dir_all(&folder);
    }
}
