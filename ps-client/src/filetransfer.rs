use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use ps_protocol::command::Command;

pub const ICON_SIZE_LIMIT: u64 = 512 * 1024;
pub const UNREACHABLE: &str = "cannot reach the file port";

pub fn icon_path(id: u32) -> String {
    format!("/icon_{id}")
}

pub fn init_download(transfer: u16, name: &str) -> Command {
    Command::new("ftinitdownload")
        .arg("clientftfid", transfer)
        .arg("name", name)
        .arg("cid", 0)
        .flag("cpw")
        .arg("seekpos", 0)
        .arg("proto", 1)
}

pub fn stop(server_transfer: u16) -> Command {
    Command::new("ftstop").arg("serverftfid", server_transfer).arg("delete", 0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    pub transfer: u16,
    pub server_transfer: u16,
    pub key: String,
    pub port: u16,
    pub size: u64,
}

pub fn parse_start(cmd: &Command) -> Option<Start> {
    let key = cmd.get("ftkey")?.to_string();
    let port: u16 = cmd.num("port")?;
    if key.is_empty() || port == 0 {
        return None;
    }
    Some(Start {
        transfer: cmd.num("clientftfid")?,
        server_transfer: cmd.num("serverftfid").unwrap_or(0),
        key,
        port,
        size: cmd.num("size")?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub transfer: u16,
    pub code: u32,
    pub message: String,
}

pub fn parse_status(cmd: &Command) -> Option<Status> {
    Some(Status {
        transfer: cmd.num("clientftfid")?,
        code: cmd.num("status").unwrap_or(0),
        message: cmd.get("msg").unwrap_or("").to_string(),
    })
}

pub fn download(addr: SocketAddr, key: &str, size: u64, limit: u64, timeout: Duration) -> Result<Vec<u8>, String> {
    if size > limit {
        return Err(format!("the file is {size} bytes, more than the {limit} allowed"));
    }
    let deadline = Instant::now() + timeout * 2;
    let mut stream =
        TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("{UNREACHABLE}: {e}"))?;
    stream.set_write_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    stream.write_all(key.as_bytes()).map_err(|e| format!("cannot send the transfer key: {e}"))?;
    let mut data = Vec::with_capacity(size as usize);
    let mut chunk = [0u8; 8192];
    while (data.len() as u64) < size {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("the download took too long".to_string());
        }
        stream.set_read_timeout(Some(left.min(timeout))).map_err(|e| e.to_string())?;
        match stream.read(&mut chunk) {
            Err(e) if left < timeout && matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
                return Err("the download took too long".to_string());
            }
            Ok(0) => break,
            Ok(n) => {
                let room = (size - data.len() as u64) as usize;
                data.extend_from_slice(&chunk[..n.min(room)]);
            }
            Err(e) => return Err(format!("the download stopped: {e}")),
        }
    }
    if data.len() as u64 != size {
        return Err(format!("got {} of {size} bytes", data.len()));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn serve(payload: Vec<u8>, expect_key: &'static str) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut key = vec![0u8; expect_key.len()];
            stream.read_exact(&mut key).unwrap();
            assert_eq!(key, expect_key.as_bytes());
            stream.write_all(&payload).unwrap();
        });
        addr
    }

    #[test]
    fn builds_the_request_and_reads_the_replies() {
        assert_eq!(
            init_download(7, &icon_path(2154984321)).build(),
            "ftinitdownload clientftfid=7 name=\\/icon_2154984321 cid=0 cpw seekpos=0 proto=1"
        );
        assert_eq!(stop(4).build(), "ftstop serverftfid=4 delete=0");
        let start = parse_start(&Command::parse(
            "notifystartdownload clientftfid=7 serverftfid=4 ftkey=7+MdQmmLmPE0b8XFx1z0pU9AGhuqZEEs port=30033 size=92 proto=1",
        ))
        .unwrap();
        assert_eq!((start.transfer, start.server_transfer, start.port, start.size), (7, 4, 30033, 92));
        assert_eq!(start.key, "7+MdQmmLmPE0b8XFx1z0pU9AGhuqZEEs");
        assert!(parse_start(&Command::parse("notifystartdownload clientftfid=7 port=30033 size=92")).is_none());
        assert!(parse_start(&Command::parse("notifystartdownload clientftfid=7 ftkey=k port=0 size=92")).is_none());
        assert!(parse_start(&Command::parse("notifystartdownload clientftfid=7 ftkey=k port=30033")).is_none());
        let status = parse_status(&Command::parse(
            "notifystatusfiletransfer clientftfid=21 status=2054 msg=invalid\\sfile\\spath size=0",
        ))
        .unwrap();
        assert_eq!((status.transfer, status.code, status.message.as_str()), (21, 2054, "invalid file path"));
        assert!(parse_status(&Command::parse("notifystatusfiletransfer status=2054")).is_none());
    }

    #[test]
    fn downloads_exactly_the_announced_bytes() {
        let payload: Vec<u8> = (0..92u8).collect();
        let addr = serve(payload.clone(), "0123456789abcdef0123456789abcdef");
        let got = download(addr, "0123456789abcdef0123456789abcdef", 92, ICON_SIZE_LIMIT, Duration::from_secs(2));
        assert_eq!(got, Ok(payload));

        let mut long: Vec<u8> = (0..200u8).collect();
        let addr = serve(long.clone(), "k");
        long.truncate(150);
        assert_eq!(download(addr, "k", 150, ICON_SIZE_LIMIT, Duration::from_secs(2)), Ok(long));

        let addr = serve(Vec::new(), "k");
        assert_eq!(download(addr, "k", 0, ICON_SIZE_LIMIT, Duration::from_secs(2)), Ok(Vec::new()));
    }

    #[test]
    fn refuses_oversized_short_and_silent_transfers() {
        let unused: SocketAddr = "127.0.0.1:9".parse().unwrap();
        assert!(download(unused, "k", ICON_SIZE_LIMIT + 1, ICON_SIZE_LIMIT, Duration::from_secs(2)).is_err());
        let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let refused = download(closed, "k", 10, ICON_SIZE_LIMIT, Duration::from_secs(3)).unwrap_err();
        assert!(refused.starts_with(UNREACHABLE), "{refused}");

        let addr = serve(vec![1, 2, 3], "k");
        let short = download(addr, "k", 10, ICON_SIZE_LIMIT, Duration::from_secs(2));
        assert_eq!(short, Err("got 3 of 10 bytes".to_string()));

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let keep = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(900));
            drop(stream);
        });
        let began = Instant::now();
        assert!(download(addr, "k", 10, ICON_SIZE_LIMIT, Duration::from_millis(300)).is_err());
        assert!(began.elapsed() < Duration::from_millis(800));
        keep.join().unwrap();
    }

    #[test]
    fn a_trickling_server_cannot_hold_the_download_open() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let feeder = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            for _ in 0..40 {
                if stream.write_all(&[7]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let began = Instant::now();
        let result = download(addr, "k", 1000, ICON_SIZE_LIMIT, Duration::from_millis(250));
        assert_eq!(result, Err("the download took too long".to_string()));
        assert!(began.elapsed() < Duration::from_millis(900), "took {:?}", began.elapsed());
        feeder.join().unwrap();
    }
}
