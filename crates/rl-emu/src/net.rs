//! Network transport for multiplayer, replacing DirectPlay's service providers.
//!
//! One machine hosts: it listens on a TCP port and relays messages between the players
//! (a star topology; sessions have at most four players). Hosts also answer UDP discovery
//! requests so other machines on the LAN (or an address given explicitly) can list the
//! session. All sockets use `TCP_NODELAY`: the game sends many small messages and the
//! original provider's acknowledgements were what made online play sluggish.

use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket},
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub const PORT: u16 = 47624;
const PROTOCOL: u8 = 1;
const MAGIC_REQ: &[u8] = b"RLD1?";
const MAGIC_REPLY: &[u8] = b"RLD1!";

#[derive(Clone, Debug, Default)]
pub struct SessionInfo {
    pub guid: [u8; 16],
    pub app_guid: [u8; 16],
    pub name: String,
    pub max_players: u32,
    pub flags: u32,
    pub players: Vec<(u32, String)>,
    /// Where the host listens (filled in on discovery).
    pub addr: Option<SocketAddr>,
}

#[derive(Debug)]
pub enum Event {
    PlayerAdd { id: u32, name: String },
    PlayerDel { id: u32 },
    Data { from: u32, to: u32, data: Vec<u8> },
    SessionLost,
}

struct Conn {
    stream: TcpStream,
    /// Players created through this connection (host side).
    players: Vec<u32>,
}

struct Core {
    hosting: bool,
    joined: bool,
    session: SessionInfo,
    next_id: u32,
    /// All known players: (id, name, is local to this process).
    players: Vec<(u32, String, bool)>,
    conns: HashMap<u32, Conn>,
    next_conn: u32,
    /// Client side: the connection to the host.
    host_stream: Option<TcpStream>,
    assigned: Option<Sender<u32>>,
    sessions: HashMap<[u8; 16], (SessionInfo, Instant)>,
    extra_hosts: Vec<String>,
    discovering: bool,
    /// Public address learned through UPnP (shown to the host).
    pub public: Option<String>,
    upnp: Option<crate::upnp::Mapping>,
}

pub struct Net {
    core: Arc<Mutex<Core>>,
    tx: Sender<Event>,
    pub rx: Receiver<Event>,
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    out.push(b.len().min(255) as u8);
    out.extend_from_slice(&b[..b.len().min(255)]);
}

fn get_str(d: &[u8], p: &mut usize) -> String {
    let n = *d.get(*p).unwrap_or(&0) as usize;
    let s = d.get(*p + 1..*p + 1 + n).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
    *p += 1 + n;
    s
}

fn get_u32(d: &[u8], p: &mut usize) -> u32 {
    let v = d.get(*p..*p + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    *p += 4;
    v
}

fn encode_session(s: &SessionInfo, tcp_port: u16) -> Vec<u8> {
    let mut o = Vec::new();
    o.extend_from_slice(&s.guid);
    o.extend_from_slice(&s.app_guid);
    put_str(&mut o, &s.name);
    o.extend_from_slice(&s.max_players.to_le_bytes());
    o.extend_from_slice(&s.flags.to_le_bytes());
    o.extend_from_slice(&tcp_port.to_le_bytes());
    o.push(s.players.len() as u8);
    for (id, n) in &s.players {
        o.extend_from_slice(&id.to_le_bytes());
        put_str(&mut o, n);
    }
    o
}

fn decode_session(d: &[u8]) -> Option<(SessionInfo, u16)> {
    if d.len() < 32 {
        return None;
    }
    let mut s = SessionInfo::default();
    s.guid.copy_from_slice(&d[..16]);
    s.app_guid.copy_from_slice(&d[16..32]);
    let mut p = 32;
    s.name = get_str(d, &mut p);
    s.max_players = get_u32(d, &mut p);
    s.flags = get_u32(d, &mut p);
    let port = d.get(p..p + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))?;
    p += 2;
    let n = *d.get(p)? as usize;
    p += 1;
    for _ in 0..n {
        let id = get_u32(d, &mut p);
        let name = get_str(d, &mut p);
        s.players.push((id, name));
    }
    Some((s, port))
}

fn write_frame(s: &mut TcpStream, payload: &[u8]) -> bool {
    let mut buf = Vec::with_capacity(payload.len() + 4);
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(payload);
    s.write_all(&buf).is_ok()
}

fn read_frame(s: &mut TcpStream) -> Option<Vec<u8>> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len).ok()?;
    let n = u32::from_le_bytes(len) as usize;
    if n > 1 << 20 {
        return None;
    }
    let mut buf = vec![0u8; n];
    s.read_exact(&mut buf).ok()?;
    Some(buf)
}

// Frame kinds.
const HELLO: u8 = 1;
const SESSION: u8 = 2;
const PLAYER_ADD: u8 = 3;
const PLAYER_DEL: u8 = 4;
const DATA: u8 = 5;
const REQ_PLAYER: u8 = 6;
const PLAYER_ASSIGN: u8 = 7;

impl Net {
    pub fn new() -> Net {
        let (tx, rx) = channel();
        let extra_hosts = std::env::var("RL_JOIN").map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
        let core = Core {
            hosting: false,
            joined: false,
            session: SessionInfo::default(),
            next_id: 1,
            players: Vec::new(),
            conns: HashMap::new(),
            next_conn: 1,
            host_stream: None,
            assigned: None,
            sessions: HashMap::new(),
            extra_hosts,
            discovering: false,
            public: None,
            upnp: None,
        };
        Net { core: Arc::new(Mutex::new(core)), tx, rx }
    }

    /// Starts looking for sessions on the LAN and at any `RL_JOIN` addresses.
    pub fn start_discovery(&self) {
        {
            let mut c = self.core.lock().unwrap();
            if c.discovering {
                return;
            }
            c.discovering = true;
        }
        let core = self.core.clone();
        thread::spawn(move || {
            let Ok(sock) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else { return };
            let _ = sock.set_broadcast(true);
            let _ = sock.set_read_timeout(Some(Duration::from_millis(250)));
            let mut last_send = Instant::now() - Duration::from_secs(10);
            let mut buf = [0u8; 2048];
            loop {
                if !core.lock().unwrap().discovering {
                    return;
                }
                if last_send.elapsed() > Duration::from_millis(1000) {
                    last_send = Instant::now();
                    let targets: Vec<String> = core.lock().unwrap().extra_hosts.clone();
                    let _ = sock.send_to(MAGIC_REQ, (Ipv4Addr::BROADCAST, PORT));
                    let _ = sock.send_to(MAGIC_REQ, (Ipv4Addr::LOCALHOST, PORT));
                    for t in targets {
                        let t = if t.contains(':') { t } else { format!("{t}:{PORT}") };
                        if let Ok(mut it) = t.to_socket_addrs() {
                            if let Some(a) = it.next() {
                                let _ = sock.send_to(MAGIC_REQ, a);
                            }
                        }
                    }
                }
                if let Ok((n, from)) = sock.recv_from(&mut buf) {
                    if buf[..n].starts_with(MAGIC_REPLY) {
                        if let Some((mut s, port)) = decode_session(&buf[MAGIC_REPLY.len()..n]) {
                            s.addr = Some(SocketAddr::new(from.ip(), port));
                            core.lock().unwrap().sessions.insert(s.guid, (s, Instant::now()));
                        }
                    }
                }
            }
        });
    }

    pub fn stop_discovery(&self) {
        self.core.lock().unwrap().discovering = false;
    }

    /// Adds an address (`host` or `host:port`) to probe for sessions, e.g. typed by the user.
    pub fn add_host(&self, host: &str) {
        let mut c = self.core.lock().unwrap();
        let h = host.trim().to_string();
        if !h.is_empty() && !c.extra_hosts.contains(&h) {
            c.extra_hosts.push(h);
        }
    }

    /// Local address other machines on the LAN can reach us at (best effort).
    pub fn local_ip() -> Option<std::net::IpAddr> {
        // Connecting a UDP socket sends nothing; it only selects the outgoing interface.
        let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
        s.connect((Ipv4Addr::new(8, 8, 8, 8), 80)).ok()?;
        Some(s.local_addr().ok()?.ip())
    }

    /// Sessions heard from in the last few seconds.
    pub fn sessions(&self) -> Vec<SessionInfo> {
        let c = self.core.lock().unwrap();
        let mut v: Vec<SessionInfo> = c.sessions.values().filter(|(_, t)| t.elapsed() < Duration::from_secs(4)).map(|(s, _)| s.clone()).collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    pub fn session_by_guid(&self, guid: &[u8; 16]) -> Option<SessionInfo> {
        self.core.lock().unwrap().sessions.get(guid).map(|(s, _)| s.clone())
    }

    /// Creates a session: listens for players and answers discovery requests.
    pub fn host(&self, mut info: SessionInfo) -> Result<(), String> {
        let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, PORT)).or_else(|_| TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0))).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        // The guest picked guidInstance; make sure it is unique per hosting run.
        if info.guid == [0; 16] {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos());
            info.guid = t.to_le_bytes();
        }
        {
            let mut c = self.core.lock().unwrap();
            c.hosting = true;
            c.session = info;
        }
        eprintln!("[net] hosting on TCP port {port}");
        // Ask the router to forward the port (in the background; it can take a few seconds).
        if std::env::var_os("RL_NO_UPNP").is_none() {
            if let Some(ip) = Net::local_ip() {
                let core = self.core.clone();
                thread::spawn(move || {
                    if let Some(m) = crate::upnp::map_port(port, ip) {
                        eprintln!("[net] UPnP: port {port} open, public address {:?}", m.external_ip);
                        let mut c = core.lock().unwrap();
                        c.public = m.external_ip.clone().map(|a| format!("{a}:{port}"));
                        c.upnp = Some(m);
                    } else {
                        eprintln!("[net] UPnP not available");
                    }
                });
            }
        }
        // Session answers for discovery.
        if let Ok(udp) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, PORT)) {
            let core = self.core.clone();
            thread::spawn(move || {
                let mut buf = [0u8; 64];
                let _ = udp.set_read_timeout(Some(Duration::from_millis(300)));
                loop {
                    {
                        let c = core.lock().unwrap();
                        if !c.hosting {
                            return;
                        }
                    }
                    if let Ok((n, from)) = udp.recv_from(&mut buf) {
                        if &buf[..n] == MAGIC_REQ {
                            let c = core.lock().unwrap();
                            let mut s = c.session.clone();
                            s.players = c.players.iter().map(|(i, n, _)| (*i, n.clone())).collect();
                            let mut reply = MAGIC_REPLY.to_vec();
                            reply.extend(encode_session(&s, port));
                            let _ = udp.send_to(&reply, from);
                        }
                    }
                }
            });
        }
        let core = self.core.clone();
        let tx = self.tx.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                if !core.lock().unwrap().hosting {
                    return;
                }
                let Ok(stream) = stream else { continue };
                let _ = stream.set_nodelay(true);
                let (core, tx) = (core.clone(), tx.clone());
                thread::spawn(move || serve_client(stream, core, tx));
            }
        });
        Ok(())
    }

    /// Joins a discovered session.
    pub fn join(&self, info: &SessionInfo) -> Result<(), String> {
        let addr = info.addr.ok_or("session has no address")?;
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|e| e.to_string())?;
        let _ = stream.set_nodelay(true);
        let mut hello = vec![HELLO, PROTOCOL];
        put_str(&mut hello, "player");
        if !write_frame(&mut stream, &hello) {
            return Err("cannot talk to host".into());
        }
        let reader = stream.try_clone().map_err(|e| e.to_string())?;
        {
            let mut c = self.core.lock().unwrap();
            c.joined = true;
            c.session = info.clone();
            c.host_stream = Some(stream);
        }
        let (core, tx) = (self.core.clone(), self.tx.clone());
        thread::spawn(move || client_reader(reader, core, tx));
        Ok(())
    }

    /// Registers a local player; blocks briefly when a host has to assign the id.
    pub fn create_player(&self, name: &str) -> Result<u32, String> {
        let hosting = self.core.lock().unwrap().hosting;
        if hosting {
            let mut c = self.core.lock().unwrap();
            let id = c.next_id;
            c.next_id += 1;
            c.players.push((id, name.to_string(), true));
            let mut f = vec![PLAYER_ADD];
            f.extend_from_slice(&id.to_le_bytes());
            put_str(&mut f, name);
            broadcast(&mut c, &f, None);
            return Ok(id);
        }
        let (tx, rx) = channel();
        {
            let mut c = self.core.lock().unwrap();
            c.assigned = Some(tx);
            let mut f = vec![REQ_PLAYER];
            put_str(&mut f, name);
            let ok = c.host_stream.as_mut().is_some_and(|s| write_frame(s, &f));
            if !ok {
                return Err("not connected".into());
            }
        }
        let id = rx.recv_timeout(Duration::from_secs(5)).map_err(|_| "host did not answer".to_string())?;
        self.core.lock().unwrap().players.push((id, name.to_string(), true));
        Ok(id)
    }

    pub fn players(&self) -> Vec<(u32, String, bool)> {
        self.core.lock().unwrap().players.clone()
    }

    pub fn session(&self) -> SessionInfo {
        let c = self.core.lock().unwrap();
        let mut s = c.session.clone();
        s.players = c.players.iter().map(|(i, n, _)| (*i, n.clone())).collect();
        s
    }

    /// Sends application data; `to == 0` means everyone else.
    pub fn send(&self, from: u32, to: u32, data: &[u8]) {
        let mut f = vec![DATA];
        f.extend_from_slice(&from.to_le_bytes());
        f.extend_from_slice(&to.to_le_bytes());
        f.extend_from_slice(data);
        let mut c = self.core.lock().unwrap();
        if c.hosting {
            if to == 0 {
                broadcast(&mut c, &f, None);
            } else if let Some(conn) = conn_of_player(&c, to) {
                if let Some(cn) = c.conns.get_mut(&conn) {
                    write_frame(&mut cn.stream, &f);
                }
            }
        } else if let Some(s) = c.host_stream.as_mut() {
            write_frame(s, &f);
        }
    }

    pub fn destroy_player(&self, id: u32) {
        let mut c = self.core.lock().unwrap();
        c.players.retain(|(i, _, _)| *i != id);
        let mut f = vec![PLAYER_DEL];
        f.extend_from_slice(&id.to_le_bytes());
        if c.hosting {
            broadcast(&mut c, &f, None);
        } else if let Some(s) = c.host_stream.as_mut() {
            write_frame(s, &f);
        }
    }

    /// Public `ip:port` once UPnP has opened the router, if it did.
    pub fn public_address(&self) -> Option<String> {
        self.core.lock().unwrap().public.clone()
    }

    pub fn close(&self) {
        let mut c = self.core.lock().unwrap();
        if let Some(m) = c.upnp.take() {
            crate::upnp::unmap(&m);
        }
        c.public = None;
        c.hosting = false;
        c.joined = false;
        c.players.clear();
        for (_, cn) in c.conns.drain() {
            let _ = cn.stream.shutdown(std::net::Shutdown::Both);
        }
        if let Some(s) = c.host_stream.take() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
        c.next_id = 1;
    }
}

fn conn_of_player(c: &Core, player: u32) -> Option<u32> {
    c.conns.iter().find(|(_, cn)| cn.players.contains(&player)).map(|(id, _)| *id)
}

/// Sends a frame to every connection except `skip`.
fn broadcast(c: &mut Core, frame: &[u8], skip: Option<u32>) {
    for (id, cn) in c.conns.iter_mut() {
        if Some(*id) != skip {
            write_frame(&mut cn.stream, frame);
        }
    }
}

fn serve_client(mut stream: TcpStream, core: Arc<Mutex<Core>>, tx: Sender<Event>) {
    let Some(hello) = read_frame(&mut stream) else { return };
    if hello.first() != Some(&HELLO) || hello.get(1) != Some(&PROTOCOL) {
        return;
    }
    let conn_id = {
        let mut c = core.lock().unwrap();
        let id = c.next_conn;
        c.next_conn += 1;
        let Ok(writer) = stream.try_clone() else { return };
        // Tell the newcomer about the session and who is already in it.
        let mut w = writer.try_clone().unwrap();
        let mut s = c.session.clone();
        s.players = c.players.iter().map(|(i, n, _)| (*i, n.clone())).collect();
        let mut f = vec![SESSION];
        f.extend(encode_session(&s, 0));
        write_frame(&mut w, &f);
        for (pid, name, _) in &c.players {
            let mut f = vec![PLAYER_ADD];
            f.extend_from_slice(&pid.to_le_bytes());
            put_str(&mut f, name);
            write_frame(&mut w, &f);
        }
        c.conns.insert(id, Conn { stream: writer, players: Vec::new() });
        id
    };
    while let Some(f) = read_frame(&mut stream) {
        let Some((&kind, body)) = f.split_first() else { continue };
        let mut c = core.lock().unwrap();
        match kind {
            REQ_PLAYER => {
                let mut p = 0;
                let name = get_str(body, &mut p);
                let id = c.next_id;
                c.next_id += 1;
                c.players.push((id, name.clone(), false));
                if let Some(cn) = c.conns.get_mut(&conn_id) {
                    cn.players.push(id);
                    let mut a = vec![PLAYER_ASSIGN];
                    a.extend_from_slice(&id.to_le_bytes());
                    write_frame(&mut cn.stream, &a);
                }
                let mut add = vec![PLAYER_ADD];
                add.extend_from_slice(&id.to_le_bytes());
                put_str(&mut add, &name);
                broadcast(&mut c, &add, Some(conn_id));
                let _ = tx.send(Event::PlayerAdd { id, name });
            }
            PLAYER_DEL => {
                let mut p = 0;
                let id = get_u32(body, &mut p);
                remove_player(&mut c, &tx, id, Some(conn_id));
            }
            DATA => {
                let mut p = 0;
                let (from, to) = (get_u32(body, &mut p), get_u32(body, &mut p));
                let data = body[p.min(body.len())..].to_vec();
                if to == 0 {
                    broadcast(&mut c, &f, Some(conn_id));
                    let _ = tx.send(Event::Data { from, to, data });
                } else if let Some(target) = conn_of_player(&c, to) {
                    if let Some(cn) = c.conns.get_mut(&target) {
                        write_frame(&mut cn.stream, &f);
                    }
                } else if c.players.iter().any(|(i, _, local)| *i == to && *local) {
                    let _ = tx.send(Event::Data { from, to, data });
                }
            }
            _ => {}
        }
    }
    // Connection ended: its players leave.
    let mut c = core.lock().unwrap();
    if let Some(cn) = c.conns.remove(&conn_id) {
        for id in cn.players {
            remove_player(&mut c, &tx, id, None);
        }
    }
}

fn remove_player(c: &mut Core, tx: &Sender<Event>, id: u32, skip: Option<u32>) {
    if !c.players.iter().any(|(i, _, _)| *i == id) {
        return;
    }
    c.players.retain(|(i, _, _)| *i != id);
    let mut f = vec![PLAYER_DEL];
    f.extend_from_slice(&id.to_le_bytes());
    broadcast(c, &f, skip);
    let _ = tx.send(Event::PlayerDel { id });
}

fn client_reader(mut stream: TcpStream, core: Arc<Mutex<Core>>, tx: Sender<Event>) {
    while let Some(f) = read_frame(&mut stream) {
        let Some((&kind, body)) = f.split_first() else { continue };
        match kind {
            SESSION => {
                if let Some((s, _)) = decode_session(body) {
                    let mut c = core.lock().unwrap();
                    let addr = c.session.addr;
                    c.session = s;
                    c.session.addr = addr;
                }
            }
            PLAYER_ADD => {
                let mut p = 0;
                let id = get_u32(body, &mut p);
                let name = get_str(body, &mut p);
                core.lock().unwrap().players.push((id, name.clone(), false));
                let _ = tx.send(Event::PlayerAdd { id, name });
            }
            PLAYER_DEL => {
                let mut p = 0;
                let id = get_u32(body, &mut p);
                core.lock().unwrap().players.retain(|(i, _, _)| *i != id);
                let _ = tx.send(Event::PlayerDel { id });
            }
            PLAYER_ASSIGN => {
                let mut p = 0;
                let id = get_u32(body, &mut p);
                if let Some(s) = core.lock().unwrap().assigned.take() {
                    let _ = s.send(id);
                }
            }
            DATA => {
                let mut p = 0;
                let (from, to) = (get_u32(body, &mut p), get_u32(body, &mut p));
                let _ = tx.send(Event::Data { from, to, data: body[p.min(body.len())..].to_vec() });
            }
            _ => {}
        }
    }
    core.lock().unwrap().joined = false;
    let _ = tx.send(Event::SessionLost);
}
