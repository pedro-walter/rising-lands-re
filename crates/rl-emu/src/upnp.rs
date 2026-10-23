//! Opens a TCP port on the home router with UPnP (IGD), so a hosted game can be reached from
//! the internet without VPNs or manual port forwarding. Best effort: many routers have UPnP off,
//! and some connections (CGNAT) have no public address at all.

use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket},
    time::Duration,
};

pub struct Mapping {
    pub external_ip: Option<String>,
    pub port: u16,
    host: String,
    control_path: String,
    service: String,
}

fn http(host: &str, req: &str) -> Option<String> {
    let addr = host.to_socket_addrs().ok()?.next()?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(4))).ok()?;
    s.write_all(req.as_bytes()).ok()?;
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    Some(String::from_utf8_lossy(&out).into_owned())
}

fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let a = s.find(open)? + open.len();
    let b = s[a..].find(close)? + a;
    Some(&s[a..b])
}

fn soap(m: &Mapping, action: &str, args: &str) -> Option<String> {
    let body = format!(
        "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{svc}\">{args}</u:{action}></s:Body></s:Envelope>",
        svc = m.service
    );
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: text/xml; charset=\"utf-8\"\r\nSOAPAction: \"{svc}#{action}\"\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
        path = m.control_path,
        host = m.host,
        svc = m.service,
        len = body.len()
    );
    let r = http(&m.host, &req)?;
    r.starts_with("HTTP/1.1 200").then_some(r)
}

/// Finds the router and maps `port` (TCP) to `local_ip:port`.
pub fn map_port(port: u16, local_ip: IpAddr) -> Option<Mapping> {
    let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    sock.set_read_timeout(Some(Duration::from_millis(800))).ok()?;
    let search = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\n\r\n";
    let mut location = None;
    for _ in 0..3 {
        let _ = sock.send_to(search.as_bytes(), SocketAddr::new(Ipv4Addr::new(239, 255, 255, 250).into(), 1900));
        let mut buf = [0u8; 2048];
        while let Ok((n, _)) = sock.recv_from(&mut buf) {
            let text = String::from_utf8_lossy(&buf[..n]).into_owned();
            if let Some(l) = text.lines().find_map(|l| l.strip_prefix("LOCATION:").or_else(|| l.strip_prefix("Location:")).or_else(|| l.strip_prefix("location:"))) {
                location = Some(l.trim().to_string());
                break;
            }
        }
        if location.is_some() {
            break;
        }
    }
    let location = location?;
    let rest = location.strip_prefix("http://")?;
    let (host, path) = rest.split_once('/').map_or((rest, "/".to_string()), |(h, p)| (h, format!("/{p}")));
    let desc = http(host, &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"))?;
    // Prefer WANIPConnection, fall back to WANPPPConnection.
    let service = ["urn:schemas-upnp-org:service:WANIPConnection:1", "urn:schemas-upnp-org:service:WANIPConnection:2", "urn:schemas-upnp-org:service:WANPPPConnection:1"]
        .into_iter()
        .find(|s| desc.contains(s))?;
    let after = &desc[desc.find(service)?..];
    let control = between(after, "<controlURL>", "</controlURL>")?;
    let control_path = if control.starts_with('/') { control.to_string() } else { format!("/{control}") };
    let mut m = Mapping { external_ip: None, port, host: host.to_string(), control_path, service: service.to_string() };
    let args = format!(
        "<NewRemoteHost></NewRemoteHost><NewExternalPort>{port}</NewExternalPort><NewProtocol>TCP</NewProtocol><NewInternalPort>{port}</NewInternalPort><NewInternalClient>{local_ip}</NewInternalClient><NewEnabled>1</NewEnabled><NewPortMappingDescription>Rising Lands</NewPortMappingDescription><NewLeaseDuration>0</NewLeaseDuration>"
    );
    soap(&m, "AddPortMapping", &args)?;
    m.external_ip = soap(&m, "GetExternalIPAddress", "").and_then(|r| between(&r, "<NewExternalIPAddress>", "</NewExternalIPAddress>").map(str::to_string));
    Some(m)
}

pub fn unmap(m: &Mapping) {
    let args = format!("<NewRemoteHost></NewRemoteHost><NewExternalPort>{}</NewExternalPort><NewProtocol>TCP</NewProtocol>", m.port);
    let _ = soap(m, "DeletePortMapping", &args);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// Serves one canned HTTP response per connection and reports the requests it saw.
    fn fake_router(responses: Vec<String>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = l.local_addr().unwrap().to_string();
        let h = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for resp in responses {
                let (mut s, _) = l.accept().unwrap();
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap();
                seen.push(String::from_utf8_lossy(&buf[..n]).into_owned());
                s.write_all(resp.as_bytes()).unwrap();
            }
            seen
        });
        (host, h)
    }

    fn mapping(host: &str) -> Mapping {
        Mapping { external_ip: None, port: 47624, host: host.into(), control_path: "/ctl".into(), service: "urn:test:1".into() }
    }

    #[test]
    fn between_extracts_the_first_delimited_slice() {
        assert_eq!(between("<a>x</a><a>y</a>", "<a>", "</a>"), Some("x"));
        assert_eq!(between("no tags", "<a>", "</a>"), None);
        assert_eq!(between("<a>open", "<a>", "</a>"), None);
        assert_eq!(between("<a></a>", "<a>", "</a>"), Some(""));
    }

    #[test]
    fn soap_posts_the_action_and_accepts_only_200() {
        let (host, h) = fake_router(vec!["HTTP/1.1 200 OK\r\n\r\n<NewExternalIPAddress>1.2.3.4</NewExternalIPAddress>".into(), "HTTP/1.1 500 Error\r\n\r\n".into()]);
        let m = mapping(&host);
        let ok = soap(&m, "GetExternalIPAddress", "<x/>").unwrap();
        assert_eq!(between(&ok, "<NewExternalIPAddress>", "</NewExternalIPAddress>"), Some("1.2.3.4"));
        assert!(soap(&m, "AddPortMapping", "").is_none());
        let seen = h.join().unwrap();
        assert!(seen[0].starts_with("POST /ctl HTTP/1.1"));
        assert!(seen[0].contains("SOAPAction: \"urn:test:1#GetExternalIPAddress\""));
        assert!(seen[0].contains("<u:GetExternalIPAddress xmlns:u=\"urn:test:1\"><x/></u:GetExternalIPAddress>"));
    }

    #[test]
    fn unmap_sends_a_delete_request_and_survives_failures() {
        let (host, h) = fake_router(vec!["HTTP/1.1 200 OK\r\n\r\n".into()]);
        unmap(&mapping(&host));
        let seen = h.join().unwrap();
        assert!(seen[0].contains("DeletePortMapping"));
        assert!(seen[0].contains("<NewExternalPort>47624</NewExternalPort><NewProtocol>TCP</NewProtocol>"));
        // Nothing listening: no panic, no hang.
        let dead = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = dead.local_addr().unwrap().to_string();
        drop(dead);
        unmap(&mapping(&host));
    }

    #[test]
    fn http_reports_unreachable_hosts_as_none() {
        assert!(http("not a host", "GET / HTTP/1.1\r\n\r\n").is_none());
    }
}
