#[path = "../src/resolve.rs"]
mod resolve;

use std::time::Instant;

use ps_client::DEFAULT_PORT;
use resolve::{order_srv, plain_step, split_host_port, srv_lookup, srv_step, tsdns_domains, tsdns_step, Found, System};

fn show(label: &str, run: impl FnOnce() -> Option<Found>) {
    let began = Instant::now();
    let found = run();
    let took = began.elapsed().as_millis();
    match found {
        Some(found) => println!("{label}: {} through {} ({took} ms)", found.addr, found.how),
        None => println!("{label}: nothing ({took} ms)"),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut host = None;
    let mut record = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--srv" => record = args.next(),
            _ if host.is_none() => host = Some(arg),
            _ => {}
        }
    }
    if let Some(record) = &record {
        let began = Instant::now();
        let records = order_srv(srv_lookup(record));
        println!("srv {record}: {} record(s) in {} ms", records.len(), began.elapsed().as_millis());
        for srv in &records {
            println!("  priority {} weight {} port {} target {}", srv.priority, srv.weight, srv.port, srv.target);
        }
    }
    let Some(host) = host else {
        if record.is_none() {
            println!("usage: resolve <name[:port]> [--srv <full record name>]");
        }
        return;
    };
    let typed = host.trim();
    let (name, port) = split_host_port(typed);
    match port {
        Some(port) => println!("name {name}, port {port} as typed"),
        None => println!("name {name}, port {DEFAULT_PORT} by default"),
    }
    println!("tsdns domains: {:?}", tsdns_domains(name));
    show("srv", || srv_step(&System, name));
    show("tsdns", || tsdns_step(&System, name, typed, port));
    show("plain", || plain_step(&System, name, port.unwrap_or(DEFAULT_PORT)));
    let began = Instant::now();
    let choice = resolve::resolve(typed, DEFAULT_PORT);
    let took = began.elapsed().as_millis();
    match choice {
        Ok(found) => println!("choice: {} through {} ({took} ms)", found.addr, found.how),
        Err(e) => println!("choice: {e} ({took} ms)"),
    }
}
