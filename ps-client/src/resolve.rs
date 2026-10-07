use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

const SRV_VOICE: &str = "_ts3._udp.";
const SRV_TSDNS: &str = "_tsdns._tcp.";
const TSDNS_PORT: u16 = 41144;
const TSDNS_CONNECT: Duration = Duration::from_secs(2);
const TSDNS_READ: Duration = Duration::from_secs(2);
const TSDNS_LIMIT: usize = 512;
const OVERALL: Duration = Duration::from_secs(4);
const GRACE: Duration = Duration::from_millis(800);

#[derive(Debug)]
pub(crate) struct Found {
    pub addr: SocketAddr,
    pub how: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Srv {
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    pub target: String,
}

pub(crate) trait Lookups: Send + Sync {
    fn srv(&self, record: &str) -> Vec<Srv>;
    fn plain(&self, name: &str, port: u16) -> Result<SocketAddr, String>;
    fn tsdns(&self, server: SocketAddr, typed: &str) -> Result<SocketAddr, String>;
}

pub(crate) struct System;

impl Lookups for System {
    fn srv(&self, record: &str) -> Vec<Srv> {
        srv_lookup(record)
    }

    fn plain(&self, name: &str, port: u16) -> Result<SocketAddr, String> {
        plain_lookup(name, port)
    }

    fn tsdns(&self, server: SocketAddr, typed: &str) -> Result<SocketAddr, String> {
        tsdns_query(server, typed)
    }
}

pub(crate) fn resolve(host: &str, default_port: u16) -> Result<Found, String> {
    resolve_with(host, default_port, Arc::new(System))
}

pub(crate) fn split_host_port(host: &str) -> (&str, Option<u16>) {
    if let Some(rest) = host.strip_prefix('[') {
        let Some((name, tail)) = rest.split_once(']') else {
            return (rest, None);
        };
        return (name, tail.strip_prefix(':').and_then(|p| p.parse().ok()));
    }
    match host.rsplit_once(':') {
        Some((name, port)) if !name.contains(':') => match port.parse() {
            Ok(port) => (name, Some(port)),
            Err(_) => (host, None),
        },
        _ => (host, None),
    }
}

pub(crate) fn tsdns_domains(name: &str) -> Vec<String> {
    if name.parse::<IpAddr>().is_ok() {
        return Vec::new();
    }
    let labels: Vec<&str> = name.split('.').collect();
    if labels.len() < 2 || labels.iter().any(|label| label.is_empty()) {
        return Vec::new();
    }
    (0..=labels.len() - 2).map(|from| labels[from..].join(".")).collect()
}

pub(crate) fn order_srv(mut records: Vec<Srv>) -> Vec<Srv> {
    records.sort_by(|a, b| a.priority.cmp(&b.priority).then(b.weight.cmp(&a.weight)));
    records
}

pub(crate) fn srv_step(look: &dyn Lookups, name: &str) -> Option<Found> {
    let record = format!("{SRV_VOICE}{name}");
    for srv in order_srv(look.srv(&record)) {
        if let Ok(addr) = look.plain(&srv.target, srv.port) {
            return Some(Found { addr, how: format!("the service record {record}") });
        }
    }
    None
}

pub(crate) fn tsdns_step(look: &dyn Lookups, name: &str, typed: &str, typed_port: Option<u16>) -> Option<Found> {
    let domains = tsdns_domains(name);
    let mut servers: Vec<(String, u16)> = Vec::new();
    for domain in &domains {
        for srv in order_srv(look.srv(&format!("{SRV_TSDNS}{domain}"))) {
            servers.push((srv.target, srv.port));
        }
    }
    if servers.is_empty() {
        servers.extend(domains.iter().map(|domain| (domain.clone(), TSDNS_PORT)));
    }
    for (server, port) in servers {
        let Ok(at) = look.plain(&server, port) else {
            continue;
        };
        let Ok(mut addr) = look.tsdns(at, typed) else {
            continue;
        };
        if let Some(port) = typed_port {
            addr.set_port(port);
        }
        return Some(Found { addr, how: format!("the name server at {server}") });
    }
    None
}

pub(crate) fn plain_step(look: &dyn Lookups, name: &str, port: u16) -> Option<Found> {
    let addr = look.plain(name, port).ok()?;
    Some(Found { addr, how: format!("a plain lookup of {name}") })
}

pub(crate) fn resolve_with(host: &str, default_port: u16, look: Arc<dyn Lookups>) -> Result<Found, String> {
    let typed = host.trim().to_string();
    let (name, typed_port) = split_host_port(&typed);
    if name.is_empty() {
        return Err("no server address given".into());
    }
    if let Ok(ip) = name.parse::<IpAddr>() {
        let addr = SocketAddr::new(ip, typed_port.unwrap_or(default_port));
        return Ok(Found { addr, how: "the address itself".into() });
    }
    let name = name.to_string();
    let (tx, rx) = mpsc::channel();
    let mut pending = 0;
    for rank in 0..3u8 {
        if rank == 0 && typed_port.is_some() {
            continue;
        }
        let look = look.clone();
        let tx = tx.clone();
        let name = name.clone();
        let typed = typed.clone();
        let spawned = std::thread::Builder::new().name("ps-resolve".into()).spawn(move || {
            let found = match rank {
                0 => srv_step(look.as_ref(), &name),
                1 => tsdns_step(look.as_ref(), &name, &typed, typed_port),
                _ => plain_step(look.as_ref(), &name, typed_port.unwrap_or(default_port)),
            };
            let _ = tx.send((rank, found));
        });
        if spawned.is_ok() {
            pending += 1;
        }
    }
    drop(tx);
    let mut deadline = Instant::now() + OVERALL;
    let mut best: Option<(u8, Found)> = None;
    while pending > 0 {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let Ok((rank, found)) = rx.recv_timeout(left) else {
            break;
        };
        pending -= 1;
        let Some(found) = found else {
            continue;
        };
        let better = match &best {
            Some((seen, _)) => rank < *seen,
            None => true,
        };
        if better {
            best = Some((rank, found));
        }
        if rank == 0 {
            break;
        }
        deadline = deadline.min(Instant::now() + GRACE);
    }
    match best {
        Some((_, found)) => Ok(found),
        None => Err(format!("cannot find the address of {name}")),
    }
}

fn plain_lookup(name: &str, port: u16) -> Result<SocketAddr, String> {
    let addrs: Vec<SocketAddr> = (name, port)
        .to_socket_addrs()
        .map_err(|e| format!("cannot resolve {name}: {e}"))?
        .collect();
    addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| addrs.first())
        .copied()
        .ok_or_else(|| format!("{name} has no address"))
}

fn tsdns_query(server: SocketAddr, typed: &str) -> Result<SocketAddr, String> {
    let mut stream = TcpStream::connect_timeout(&server, TSDNS_CONNECT)
        .map_err(|e| format!("cannot reach the name server at {server}: {e}"))?;
    stream.set_write_timeout(Some(TSDNS_READ)).map_err(|e| e.to_string())?;
    stream
        .write_all(typed.as_bytes())
        .map_err(|e| format!("cannot ask the name server at {server}: {e}"))?;
    let deadline = Instant::now() + TSDNS_READ;
    let mut answer = Vec::new();
    let mut chunk = [0u8; 128];
    while answer.len() < TSDNS_LIMIT {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(format!("the name server at {server} took too long"));
        }
        stream.set_read_timeout(Some(left)).map_err(|e| e.to_string())?;
        let room = (TSDNS_LIMIT - answer.len()).min(chunk.len());
        match stream.read(&mut chunk[..room]) {
            Ok(0) => break,
            Ok(n) => answer.extend_from_slice(&chunk[..n]),
            Err(e) => return Err(format!("the name server at {server} gave no answer: {e}")),
        }
    }
    let answer = String::from_utf8_lossy(&answer);
    let answer = answer.trim();
    if answer.is_empty() {
        return Err(format!("the name server at {server} gave an empty answer"));
    }
    if answer.starts_with("404") {
        return Err(format!("the name server at {server} does not know {typed}"));
    }
    let shown: String = answer.chars().take(60).collect();
    answer.parse().map_err(|_| format!("the name server at {server} answered {shown}, which is not an address"))
}

#[cfg(windows)]
mod dns {
    use super::Srv;

    const DNS_TYPE_SRV: u16 = 33;
    const DNS_QUERY_STANDARD: u32 = 0;
    const DNS_FREE_RECORD_LIST: u32 = 1;
    const MAX_RECORDS: usize = 32;
    const MAX_NAME: usize = 255;

    #[repr(C)]
    struct SrvData {
        target: *const u16,
        priority: u16,
        weight: u16,
        port: u16,
        _pad: u16,
    }

    #[repr(C)]
    struct Record {
        next: *mut Record,
        _name: *const u16,
        rtype: u16,
        _data_length: u16,
        _flags: u32,
        _ttl: u32,
        _reserved: u32,
        data: SrvData,
    }

    #[link(name = "dnsapi")]
    extern "system" {
        fn DnsQuery_W(
            name: *const u16,
            rtype: u16,
            options: u32,
            servers: *mut core::ffi::c_void,
            result: *mut *mut Record,
            reserved: *mut core::ffi::c_void,
        ) -> i32;
        fn DnsFree(data: *mut core::ffi::c_void, free_type: u32);
    }

    pub(crate) fn srv_lookup(record: &str) -> Vec<Srv> {
        let mut wide: Vec<u16> = record.encode_utf16().collect();
        wide.push(0);
        let mut found = Vec::new();
        let mut head: *mut Record = std::ptr::null_mut();
        let status = unsafe {
            DnsQuery_W(
                wide.as_ptr(),
                DNS_TYPE_SRV,
                DNS_QUERY_STANDARD,
                std::ptr::null_mut(),
                &mut head,
                std::ptr::null_mut(),
            )
        };
        if status != 0 || head.is_null() {
            return found;
        }
        unsafe {
            let mut at: *const Record = head;
            while !at.is_null() && found.len() < MAX_RECORDS {
                let entry = &*at;
                if entry.rtype == DNS_TYPE_SRV && !entry.data.target.is_null() {
                    let target = entry.data.target;
                    let mut len = 0;
                    while len < MAX_NAME && *target.add(len) != 0 {
                        len += 1;
                    }
                    found.push(Srv {
                        priority: entry.data.priority,
                        weight: entry.data.weight,
                        port: entry.data.port,
                        target: String::from_utf16_lossy(std::slice::from_raw_parts(target, len)),
                    });
                }
                at = entry.next;
            }
            DnsFree(head.cast(), DNS_FREE_RECORD_LIST);
        }
        found
    }
}

#[cfg(not(windows))]
mod dns {
    use super::Srv;

    pub(crate) fn srv_lookup(_record: &str) -> Vec<Srv> {
        Vec::new()
    }
}

pub(crate) use dns::srv_lookup;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::net::TcpListener;

    #[derive(Default)]
    struct Fake {
        records: HashMap<String, Vec<Srv>>,
        addrs: HashMap<String, IpAddr>,
        answers: HashMap<String, SocketAddr>,
        slow_srv: Option<Duration>,
        slow_plain: Option<Duration>,
    }

    impl Fake {
        fn hold(stall: Option<Duration>) {
            if let Some(stall) = stall {
                std::thread::sleep(stall);
            }
        }
    }

    impl Lookups for Fake {
        fn srv(&self, record: &str) -> Vec<Srv> {
            Self::hold(self.slow_srv);
            self.records.get(record).cloned().unwrap_or_default()
        }

        fn plain(&self, name: &str, port: u16) -> Result<SocketAddr, String> {
            Self::hold(self.slow_plain);
            let ip = self.addrs.get(name).copied().ok_or_else(|| format!("no address for {name}"))?;
            Ok(SocketAddr::new(ip, port))
        }

        fn tsdns(&self, server: SocketAddr, _typed: &str) -> Result<SocketAddr, String> {
            self.answers.get(&server.to_string()).copied().ok_or_else(|| "no answer".to_string())
        }
    }

    fn known() -> Fake {
        let mut fake = Fake::default();
        fake.records.insert(
            "_ts3._udp.example.org".to_string(),
            vec![Srv { priority: 0, weight: 0, port: 2001, target: "voice.example.org".to_string() }],
        );
        fake.records.insert(
            "_tsdns._tcp.example.org".to_string(),
            vec![Srv { priority: 0, weight: 0, port: TSDNS_PORT, target: "tsdns.example.org".to_string() }],
        );
        fake.addrs.insert("voice.example.org".to_string(), "203.0.113.1".parse().unwrap());
        fake.addrs.insert("tsdns.example.org".to_string(), "192.0.2.5".parse().unwrap());
        fake.addrs.insert("example.org".to_string(), "203.0.113.3".parse().unwrap());
        fake.answers.insert("192.0.2.5:41144".to_string(), "203.0.113.2:30033".parse().unwrap());
        fake
    }

    fn serve(answer: Option<&'static str>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut asked = [0u8; 128];
            let _ = stream.read(&mut asked);
            match answer {
                Some(text) => {
                    let _ = stream.write_all(text.as_bytes());
                }
                None => std::thread::sleep(Duration::from_secs(10)),
            }
        });
        addr
    }

    #[test]
    fn names_ports_and_bracketed_addresses_are_split() {
        assert_eq!(split_host_port("ts.example.com:10000"), ("ts.example.com", Some(10000)));
        assert_eq!(split_host_port("ts.example.com"), ("ts.example.com", None));
        assert_eq!(split_host_port("ts.example.com:abc"), ("ts.example.com:abc", None));
        assert_eq!(split_host_port("127.0.0.1:1234"), ("127.0.0.1", Some(1234)));
        assert_eq!(split_host_port("::1"), ("::1", None));
        assert_eq!(split_host_port("[::1]"), ("::1", None));
        assert_eq!(split_host_port("[::1]:4321"), ("::1", Some(4321)));
        assert_eq!(split_host_port("[::1"), ("::1", None));
        assert_eq!(split_host_port(""), ("", None));
    }

    #[test]
    fn an_ip_literal_is_used_as_it_is() {
        let nothing = Arc::new(Fake::default());
        let at = |host: &str| resolve_with(host, 9987, nothing.clone()).unwrap();
        assert_eq!(at("127.0.0.1").addr, "127.0.0.1:9987".parse().unwrap());
        assert_eq!(at("127.0.0.1").how, "the address itself");
        assert_eq!(at(" 127.0.0.1:1234 ").addr, "127.0.0.1:1234".parse().unwrap());
        assert_eq!(at("[::1]").addr, "[::1]:9987".parse().unwrap());
        assert_eq!(at("::1").addr, "[::1]:9987".parse().unwrap());
        assert_eq!(at("[::1]:4321").addr, "[::1]:4321".parse().unwrap());
        assert!(resolve_with("", 9987, nothing.clone()).is_err());
        assert!(resolve_with("   ", 9987, nothing).is_err());
        assert!(plain_lookup("localhost", 9987).unwrap().ip().is_loopback());
    }

    #[test]
    fn the_name_server_is_asked_for_the_name_and_for_its_parent_domains() {
        assert_eq!(tsdns_domains("a.b.example.org"), ["a.b.example.org", "b.example.org", "example.org"]);
        assert_eq!(tsdns_domains("example.org"), ["example.org"]);
        assert!(tsdns_domains("localhost").is_empty());
        assert!(tsdns_domains("192.0.2.10").is_empty());
        assert!(tsdns_domains("::1").is_empty());
        assert!(tsdns_domains("").is_empty());
    }

    #[test]
    fn service_records_are_tried_by_priority_then_weight_and_keep_their_own_port() {
        let records = vec![
            Srv { priority: 10, weight: 90, port: 1001, target: "last.example.org".to_string() },
            Srv { priority: 1, weight: 5, port: 1002, target: "second.example.org".to_string() },
            Srv { priority: 1, weight: 50, port: 1003, target: "first.example.org".to_string() },
        ];
        let order: Vec<String> = order_srv(records.clone()).into_iter().map(|srv| srv.target).collect();
        assert_eq!(order, ["first.example.org", "second.example.org", "last.example.org"]);
        let mut fake = Fake::default();
        fake.records.insert("_ts3._udp.example.org".to_string(), records);
        fake.addrs.insert("second.example.org".to_string(), "203.0.113.7".parse().unwrap());
        let found = srv_step(&fake, "example.org").unwrap();
        assert_eq!(found.addr, "203.0.113.7:1002".parse().unwrap());
        assert_eq!(found.how, "the service record _ts3._udp.example.org");
        assert!(srv_step(&Fake::default(), "example.org").is_none());
    }

    #[test]
    fn the_service_record_wins_unless_a_port_was_typed() {
        let found = resolve_with("example.org", 9987, Arc::new(known())).unwrap();
        assert_eq!(found.addr, "203.0.113.1:2001".parse().unwrap());
        assert_eq!(found.how, "the service record _ts3._udp.example.org");
        let typed = resolve_with("example.org:7777", 9987, Arc::new(known())).unwrap();
        assert_eq!(typed.addr, "203.0.113.2:7777".parse().unwrap());
        assert_eq!(typed.how, "the name server at tsdns.example.org");
        let mut plain = known();
        plain.records.remove("_tsdns._tcp.example.org");
        let typed = resolve_with("example.org:7777", 9987, Arc::new(plain)).unwrap();
        assert_eq!(typed.addr, "203.0.113.3:7777".parse().unwrap());
    }

    #[test]
    fn the_name_server_wins_over_the_plain_lookup_and_a_typed_port_wins_over_its_answer() {
        let mut fake = known();
        fake.records.remove("_ts3._udp.example.org");
        let found = resolve_with("example.org", 9987, Arc::new(fake)).unwrap();
        assert_eq!(found.addr, "203.0.113.2:30033".parse().unwrap());
        assert_eq!(found.how, "the name server at tsdns.example.org");
        let mut fake = known();
        fake.records.remove("_ts3._udp.example.org");
        let typed = resolve_with("example.org:7777", 9987, Arc::new(fake)).unwrap();
        assert_eq!(typed.addr, "203.0.113.2:7777".parse().unwrap());
    }

    #[test]
    fn a_parent_domain_is_asked_on_port_41144_when_it_has_no_service_record() {
        let mut fake = known();
        fake.records.clear();
        fake.answers.insert("203.0.113.3:41144".to_string(), "203.0.113.9:30033".parse().unwrap());
        let found = resolve_with("ts.example.org", 9987, Arc::new(fake)).unwrap();
        assert_eq!(found.addr, "203.0.113.9:30033".parse().unwrap());
        assert_eq!(found.how, "the name server at example.org");
    }

    #[test]
    fn the_plain_lookup_is_the_last_resort() {
        let mut fake = known();
        fake.records.clear();
        fake.answers.clear();
        let found = resolve_with("example.org:7777", 9987, Arc::new(fake)).unwrap();
        assert_eq!(found.addr, "203.0.113.3:7777".parse().unwrap());
        assert_eq!(found.how, "a plain lookup of example.org");
    }

    #[test]
    fn a_name_that_nobody_knows_gives_one_sentence_naming_the_host() {
        let err = resolve_with("ts.example.invalid:7777", 9987, Arc::new(Fake::default())).unwrap_err();
        assert_eq!(err, "cannot find the address of ts.example.invalid");
        assert!(!err.contains('\n'));
    }

    #[test]
    fn a_working_plain_lookup_is_not_held_up_by_the_silent_steps() {
        let mut fake = known();
        fake.records.clear();
        fake.slow_srv = Some(Duration::from_secs(30));
        let began = Instant::now();
        let found = resolve_with("example.org", 9987, Arc::new(fake)).unwrap();
        assert_eq!(found.how, "a plain lookup of example.org");
        assert!(began.elapsed() < Duration::from_secs(2), "took {:?}", began.elapsed());
    }

    #[test]
    fn lookups_that_never_answer_still_give_up_within_five_seconds() {
        let fake = Fake {
            slow_srv: Some(Duration::from_secs(30)),
            slow_plain: Some(Duration::from_secs(30)),
            ..Fake::default()
        };
        let began = Instant::now();
        assert!(resolve_with("example.org", 9987, Arc::new(fake)).is_err());
        assert!(began.elapsed() < Duration::from_secs(5), "took {:?}", began.elapsed());
    }

    #[test]
    fn the_name_server_exchange_reads_one_answer_and_gives_up_on_silence() {
        let good = tsdns_query(serve(Some("203.0.113.4:30033")), "ts.example.org").unwrap();
        assert_eq!(good, "203.0.113.4:30033".parse().unwrap());
        let missing = tsdns_query(serve(Some("404")), "ts.example.org").unwrap_err();
        assert!(missing.contains("does not know ts.example.org"), "{missing}");
        assert!(tsdns_query(serve(Some("not an address at all")), "ts.example.org").is_err());
        assert!(tsdns_query(serve(Some("")), "ts.example.org").is_err());
        let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        assert!(tsdns_query(closed, "ts.example.org").is_err());
        let began = Instant::now();
        assert!(tsdns_query(serve(None), "ts.example.org").is_err());
        assert!(began.elapsed() < TSDNS_CONNECT + TSDNS_READ, "took {:?}", began.elapsed());
    }
}
