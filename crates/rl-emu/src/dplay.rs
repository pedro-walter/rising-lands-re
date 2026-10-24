//! DirectPlay (IDirectPlay2/3) on top of [`crate::net`].
//!
//! The game only uses a small part of DirectPlay: provider and session enumeration, Open,
//! CreatePlayer, Send/Receive and the system messages for players coming and going.

use crate::{
    machine::*,
    net::{Event, Net, SessionInfo},
};
use std::collections::VecDeque;

/// DPSPGUID_TCPIP: the only provider we offer.
const GUID_TCPIP: [u8; 16] = [0xe0, 0x5e, 0xe9, 0x36, 0x77, 0x85, 0xcf, 0x11, 0x96, 0x0c, 0x00, 0x80, 0xc7, 0x53, 0x4e, 0x82];

const DP_OK: u32 = 0;
const DPERR_BUFFERTOOSMALL: u32 = 0x8877_001e;
const DPERR_NOMESSAGES: u32 = 0x8877_00be;
const E_FAIL: u32 = 0x8000_4005;

pub struct Dp {
    pub net: Net,
    pub msgs: VecDeque<(u32, u32, Vec<u8>)>,
    pub local_player: Option<u32>,
    /// The public address was already announced to the user.
    pub announced_public: bool,
    /// Already asked the user for a host address for this join screen.
    pub asked_host: bool,
}

impl Dp {
    pub fn new() -> Dp {
        Dp { net: Net::new(), msgs: VecDeque::new(), local_player: None, asked_host: false, announced_public: false }
    }

    /// Moves network events into the receive queue as DirectPlay messages.
    fn pump(&mut self) {
        while let Ok(ev) = self.net.rx.try_recv() {
            match ev {
                Event::PlayerAdd { id, name } => {
                    // DPMSG_CREATEPLAYERORGROUP; strings are placed after the 48-byte header.
                    let mut m = vec![0u8; 48];
                    m[0..4].copy_from_slice(&3u32.to_le_bytes());
                    m[4..8].copy_from_slice(&1u32.to_le_bytes());
                    m[8..12].copy_from_slice(&id.to_le_bytes());
                    m[12..16].copy_from_slice(&(self.net.players().len() as u32).to_le_bytes());
                    m[24..28].copy_from_slice(&16u32.to_le_bytes());
                    m.extend_from_slice(name.as_bytes());
                    m.push(0);
                    self.msgs.push_back((0, 0, m));
                }
                Event::PlayerDel { id } => {
                    let mut m = vec![0u8; 44];
                    m[0..4].copy_from_slice(&5u32.to_le_bytes());
                    m[4..8].copy_from_slice(&1u32.to_le_bytes());
                    m[8..12].copy_from_slice(&id.to_le_bytes());
                    m[20..24].copy_from_slice(&16u32.to_le_bytes());
                    self.msgs.push_back((0, 0, m));
                }
                Event::Data { from, to, data } => self.msgs.push_back((from, to, data)),
                Event::SessionLost => {
                    let mut m = vec![0u8; 8];
                    m[0..4].copy_from_slice(&0x31u32.to_le_bytes());
                    self.msgs.push_back((0, 0, m));
                }
            }
        }
    }
}

fn dp<'a>(uc: &'a mut Uc) -> &'a mut Dp {
    uc.get_data_mut().com.dplay.get_or_insert_with(Dp::new)
}

/// Writes a DPSESSIONDESC2 (80 bytes) followed by the session name; returns the total size.
fn write_session_desc(uc: &mut Uc, at: u32, s: &SessionInfo) -> u32 {
    let mut b = vec![0u8; 80];
    b[0..4].copy_from_slice(&80u32.to_le_bytes());
    b[4..8].copy_from_slice(&s.flags.to_le_bytes());
    b[8..24].copy_from_slice(&s.guid);
    b[24..40].copy_from_slice(&s.app_guid);
    b[40..44].copy_from_slice(&s.max_players.to_le_bytes());
    b[44..48].copy_from_slice(&(s.players.len() as u32).to_le_bytes());
    b[48..52].copy_from_slice(&(at + 80).to_le_bytes());
    b.extend_from_slice(s.name.as_bytes());
    b.push(0);
    wr_bytes(uc, at, &b);
    b.len() as u32
}

fn read_guid(uc: &Uc, p: u32) -> [u8; 16] {
    let mut g = [0u8; 16];
    if p != 0 {
        g.copy_from_slice(&rd_bytes(uc, p, 16));
    }
    g
}

/// `DirectPlayCreate(lpGUID, lplpDP, pUnk)`.
pub fn create(uc: &mut Uc, out: u32) -> u32 {
    let o = crate::com::new_dplay_object(uc);
    wr32(uc, out, o);
    DP_OK
}

/// `DirectPlayEnumerate(cb, ctx)`: one provider, TCP/IP.
pub fn enumerate(uc: &mut Uc, cb: u32, ctx: u32, resume: u32) -> Option<u32> {
    let guid = uc.get_data_mut().alloc(16);
    wr_bytes(uc, guid, &GUID_TCPIP);
    let name = uc.get_data_mut().alloc(48);
    wr_cstr(uc, name, "Internet / LAN (TCP/IP)", 48);
    stop_with(
        uc,
        Action::Call { target: cb, args: vec![guid, name, 6, 0, ctx], resume, set_eax: Some(DP_OK), rest: Vec::new(), temps: vec![guid, name] },
    );
    None
}

/// Methods of the DirectPlay interface. `resume` is the stub address (for guest callbacks).
pub fn call_method(uc: &mut Uc, name: &str, a: &[u32; 14], resume: u32) -> Option<u32> {
    Some(match name {
        "Close" => {
            dp(uc).asked_host = false;
            dp(uc).net.close();
            DP_OK
        }
        "CreatePlayer" => {
            // (this, &id, DPNAME*, event, data, size, flags)
            let short = if a[2] != 0 { rd32(uc, a[2] + 8) } else { 0 };
            let player_name = rd_cstr(uc, short);
            match dp(uc).net.create_player(&player_name) {
                Ok(id) => {
                    wr32(uc, a[1], id);
                    dp(uc).local_player = Some(id);
                    DP_OK
                }
                Err(e) => {
                    eprintln!("[net] CreatePlayer failed: {e}");
                    E_FAIL
                }
            }
        }
        "DestroyPlayer" => {
            dp(uc).net.destroy_player(a[1]);
            DP_OK
        }
        "EnumSessions" => {
            // (this, DPSESSIONDESC2*, timeout, cb, ctx, flags)
            let app = read_guid(uc, if a[1] != 0 { a[1] + 24 } else { 0 });
            let d = dp(uc);
            d.net.start_discovery();
            // The first call needs a moment for replies to arrive.
            let t0 = std::time::Instant::now();
            while d.net.sessions().is_empty() && t0.elapsed() < std::time::Duration::from_millis(700) {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let sessions: Vec<SessionInfo> = d.net.sessions().into_iter().filter(|s| app == [0; 16] || s.app_guid == app).collect();
            if sessions.is_empty() && !d.asked_host {
                // Nobody answered on the LAN: let the user type the host's address.
                d.asked_host = true;
                uc.get_data_mut().ui.want_host_address = true;
            } else if !sessions.is_empty() {
                d.asked_host = false;
            }
            let d = dp(uc);
            let _ = d;
            let timeout = uc.get_data_mut().alloc(4);
            wr32(uc, timeout, 0);
            let mut calls = Vec::new();
            let mut temps = vec![timeout];
            for s in &sessions {
                let buf = uc.get_data_mut().alloc(80 + s.name.len() as u32 + 1);
                write_session_desc(uc, buf, s);
                temps.push(buf);
                calls.push((a[3], vec![buf, timeout, 0, a[4]]));
            }
            // Final call with DPESC_TIMEDOUT.
            calls.push((a[3], vec![0, timeout, 1, a[4]]));
            let (target, args) = calls.remove(0);
            stop_with(uc, Action::Call { target, args, resume, set_eax: Some(DP_OK), rest: calls, temps });
            return None;
        }
        "EnumPlayers" => {
            // (this, guid, cb, ctx, flags); flags & 0x80: players of another (discovered) session.
            let players: Vec<(u32, String)> = if a[4] & 0x80 != 0 {
                let g = read_guid(uc, a[1]);
                dp(uc).net.session_by_guid(&g).map(|s| s.players).unwrap_or_default()
            } else {
                dp(uc).net.players().into_iter().map(|(i, n, _)| (i, n)).collect()
            };
            if players.is_empty() {
                return Some(DP_OK);
            }
            let mut calls = Vec::new();
            let mut temps = Vec::new();
            for (id, name) in players {
                let buf = uc.get_data_mut().alloc(16 + name.len() as u32 + 1);
                let mut b = vec![0u8; 16];
                b[0..4].copy_from_slice(&16u32.to_le_bytes());
                b[8..12].copy_from_slice(&(buf + 16).to_le_bytes());
                b.extend_from_slice(name.as_bytes());
                b.push(0);
                wr_bytes(uc, buf, &b);
                temps.push(buf);
                calls.push((a[2], vec![id, 1, buf, 0, a[3]]));
            }
            let (target, args) = calls.remove(0);
            stop_with(uc, Action::Call { target, args, resume, set_eax: Some(DP_OK), rest: calls, temps });
            return None;
        }
        "GetSessionDesc" => {
            // (this, buf, &size)
            let s = dp(uc).net.session();
            let need = 80 + s.name.len() as u32 + 1;
            let have = rd32(uc, a[2]);
            wr32(uc, a[2], need);
            if have < need {
                DPERR_BUFFERTOOSMALL
            } else {
                write_session_desc(uc, a[1], &s);
                DP_OK
            }
        }
        "Open" => {
            // (this, DPSESSIONDESC2*, flags): DPOPEN_JOIN = 1, DPOPEN_CREATE = 2
            let (flags, sd) = (a[2], a[1]);
            let info = SessionInfo {
                guid: read_guid(uc, sd + 8),
                app_guid: read_guid(uc, sd + 24),
                name: rd_cstr(uc, rd32(uc, sd + 48)),
                max_players: rd32(uc, sd + 40).max(2),
                flags: rd32(uc, sd + 4),
                ..Default::default()
            };
            if flags & 2 != 0 || a[2] == 0 {
                match dp(uc).net.host(info) {
                    Ok(()) => {
                        let ip = Net::local_ip().map_or_else(|| "?".to_string(), |i| i.to_string());
                        uc.get_data_mut().ui.toast(format!("SESSAO CRIADA - OUTROS JOGADORES ENTRAM COM: {ip}"));
                        DP_OK
                    }
                    Err(e) => {
                        eprintln!("[net] cannot host: {e}");
                        E_FAIL
                    }
                }
            } else {
                let found = dp(uc).net.session_by_guid(&info.guid);
                match found.map(|s| dp(uc).net.join(&s)) {
                    Some(Ok(())) => DP_OK,
                    Some(Err(e)) => {
                        eprintln!("[net] cannot join: {e}");
                        E_FAIL
                    }
                    None => E_FAIL,
                }
            }
        }
        "Receive" => {
            // (this, &from, &to, flags, data, &size)
            let d = dp(uc);
            if !d.announced_public {
                if let Some(p) = d.net.public_address() {
                    d.announced_public = true;
                    uc.get_data_mut().ui.toast(format!("ENDERECO PUBLICO: {p}"));
                }
            }
            let d = dp(uc);
            d.pump();
            let Some((from, to, data)) = d.msgs.front().cloned() else { return Some(DPERR_NOMESSAGES) };
            if std::env::var_os("RL_NETLOG").is_some() {
                eprintln!("[net] receive: next message from={from} to={to} len={} buffer={}", data.len(), rd32(uc, a[5]));
            }
            let have = rd32(uc, a[5]);
            let mut data = data;
            // System messages carry pointers to their strings: fix them up for this buffer.
            if from == 0 && data.len() >= 48 && u32::from_le_bytes([data[0], data[1], data[2], data[3]]) == 3 {
                data[32..36].copy_from_slice(&(a[4] + 48).to_le_bytes());
                data[36..40].copy_from_slice(&(a[4] + 48).to_le_bytes());
            }
            wr32(uc, a[5], data.len() as u32);
            if (have as usize) < data.len() {
                return Some(DPERR_BUFFERTOOSMALL);
            }
            wr_bytes(uc, a[4], &data);
            wr32(uc, a[1], from);
            wr32(uc, a[2], to);
            if std::env::var_os("RL_NETLOG").is_some() {
                eprintln!("[net] recv from={from} to={to} len={} {:02x?}", data.len(), &data[..data.len().min(24)]);
            }
            if a[3] & 0x20 == 0 {
                dp(uc).msgs.pop_front();
            }
            DP_OK
        }
        "Send" => {
            // (this, from, to, flags, data, size)
            let data = rd_bytes(uc, a[4], a[5] as usize);
            if std::env::var_os("RL_NETLOG").is_some() {
                eprintln!("[net] send from={} to={} len={} {:02x?}", a[1], a[2], data.len(), &data[..data.len().min(24)]);
            }
            dp(uc).net.send(a[1], a[2], &data);
            DP_OK
        }
        "GetCaps" => {
            // DPCAPS: size, flags, timeout, maxbuffersize, maxqueuesize, maxplayers, hundredbaud, latency...
            let size = rd32(uc, a[1]);
            if size >= 40 {
                wr32(uc, a[1] + 4, 0x0000_0001);
                wr32(uc, a[1] + 8, 500);
                wr32(uc, a[1] + 12, 1000);
                wr32(uc, a[1] + 20, 4);
                wr32(uc, a[1] + 24, 10000);
                wr32(uc, a[1] + 28, 1);
            }
            DP_OK
        }
        "GetMessageCount" => {
            let d = dp(uc);
            d.pump();
            let n = d.msgs.len() as u32;
            wr32(uc, a[2], n);
            DP_OK
        }
        "InitializeConnection" | "SetSessionDesc" | "SetPlayerName" | "SetPlayerData" | "EnableNewPlayers" | "Initialize" => DP_OK,
        _ => {
            eprintln!("[dplay] unimplemented IDirectPlay::{name}({:#x}, {:#x}, {:#x}, {:#x})", a[0], a[1], a[2], a[3]);
            E_FAIL
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::testing::com_uc;

    fn call(uc: &mut Uc, name: &str, a: &[u32]) -> Option<u32> {
        let mut args = [0u32; 14];
        args[1..1 + a.len()].copy_from_slice(a);
        call_method(uc, name, &args, 0x7f00_0100)
    }

    fn scratch(uc: &mut Uc, n: u32) -> u32 {
        zeroed_alloc(uc, n)
    }

    fn take_call(uc: &mut Uc) -> (u32, Vec<u32>, Vec<(u32, Vec<u32>)>, Option<u32>) {
        match uc.get_data_mut().pending.take() {
            Some(Action::Call { target, args, rest, set_eax, .. }) => (target, args, rest, set_eax),
            _ => panic!("no guest callback queued"),
        }
    }

    #[test]
    fn create_registers_an_object_and_enumerate_offers_tcpip() {
        let mut uc = com_uc();
        let out = scratch(&mut uc, 4);
        assert_eq!(create(&mut uc, out), DP_OK);
        assert!(matches!(uc.get_data().com.objs[&rd32(&uc, out)], crate::com::Obj::DPlay));
        assert!(enumerate(&mut uc, 0x40_1000, 0xc0de, 0x7f00_0100).is_none());
        let (target, args, rest, eax) = take_call(&mut uc);
        assert_eq!((target, args[2], args[4], eax, rest.len()), (0x40_1000, 6, 0xc0de, Some(DP_OK), 0));
        assert_eq!(rd_bytes(&uc, args[0], 16), GUID_TCPIP);
        assert!(rd_cstr(&uc, args[1]).contains("TCP/IP"));
    }

    #[test]
    fn caps_and_trivial_methods() {
        let mut uc = com_uc();
        let caps = scratch(&mut uc, 64);
        wr32(&mut uc, caps, 40);
        assert_eq!(call(&mut uc, "GetCaps", &[caps]), Some(DP_OK));
        assert_eq!((rd32(&uc, caps + 20), rd32(&uc, caps + 24)), (4, 10000));
        let small = scratch(&mut uc, 64);
        wr32(&mut uc, small, 8);
        call(&mut uc, "GetCaps", &[small]);
        assert_eq!(rd32(&uc, small + 20), 0, "structures older than 40 bytes are left alone");
        for m in ["InitializeConnection", "SetSessionDesc", "SetPlayerName", "SetPlayerData", "EnableNewPlayers", "Initialize"] {
            assert_eq!(call(&mut uc, m, &[]), Some(DP_OK), "{m}");
        }
        assert_eq!(call(&mut uc, "Frobnicate", &[]), Some(E_FAIL));
        assert_eq!(call(&mut uc, "Send", &[1, 2, 0x40_2000, 4]), Some(DP_OK), "sending with no session is dropped quietly");
        assert_eq!(call(&mut uc, "DestroyPlayer", &[9]), Some(DP_OK));
    }

    #[test]
    fn player_and_session_calls_fail_cleanly_without_a_session() {
        let mut uc = com_uc();
        let (id, name) = (scratch(&mut uc, 4), scratch(&mut uc, 16));
        wr_cstr(&mut uc, 0x40_2000, "alice", 16);
        wr32(&mut uc, name + 8, 0x40_2000);
        assert_eq!(call(&mut uc, "CreatePlayer", &[id, name]), Some(E_FAIL));
        assert_eq!(call(&mut uc, "CreatePlayer", &[id, 0]), Some(E_FAIL));
        // Joining a session nobody advertised.
        let sd = scratch(&mut uc, 80);
        wr32(&mut uc, sd + 8, 0x1234_5678);
        assert_eq!(call(&mut uc, "Open", &[sd, 1]), Some(E_FAIL));
        // Session description: the buffer size is reported either way.
        let (buf, size) = (scratch(&mut uc, 128), scratch(&mut uc, 4));
        wr32(&mut uc, size, 10);
        assert_eq!(call(&mut uc, "GetSessionDesc", &[buf, size]), Some(DPERR_BUFFERTOOSMALL));
        assert_eq!(rd32(&uc, size), 81);
        wr32(&mut uc, size, 128);
        assert_eq!(call(&mut uc, "GetSessionDesc", &[buf, size]), Some(DP_OK));
        assert_eq!(rd32(&uc, buf), 80);
        assert_eq!(call(&mut uc, "Close", &[]), Some(DP_OK));
        // No players: EnumPlayers returns at once without calling the guest.
        assert_eq!(call(&mut uc, "EnumPlayers", &[0, 0x40_1000, 0, 0]), Some(DP_OK));
        assert!(uc.get_data().pending.is_none());
    }

    #[test]
    fn enum_sessions_with_nothing_found_asks_for_a_host_address_once() {
        let _serial = crate::net::NET_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut uc = com_uc();
        assert!(call(&mut uc, "EnumSessions", &[0, 0, 0x40_1000, 0xabc]).is_none());
        let (target, args, rest, eax) = take_call(&mut uc);
        // Only the final DPESC_TIMEDOUT callback: no session, flag 1.
        assert_eq!((target, args[0], args[2], args[3], eax, rest.len()), (0x40_1000, 0, 1, 0xabc, Some(DP_OK), 0));
        assert!(uc.get_data().ui.want_host_address);
        uc.get_data_mut().ui.want_host_address = false;
        call(&mut uc, "EnumSessions", &[0, 0, 0x40_1000, 0xabc]);
        take_call(&mut uc);
        assert!(!uc.get_data().ui.want_host_address, "not asked a second time for the same screen");
        call(&mut uc, "Close", &[]);
        call(&mut uc, "EnumSessions", &[0, 0, 0x40_1000, 0xabc]);
        take_call(&mut uc);
        assert!(uc.get_data().ui.want_host_address, "asked again after the session is closed");
    }

    #[test]
    fn receive_turns_network_events_into_directplay_messages() {
        let mut uc = com_uc();
        let (from, to, size, buf) = (scratch(&mut uc, 4), scratch(&mut uc, 4), scratch(&mut uc, 4), scratch(&mut uc, 256));
        let recv = |uc: &mut Uc, flags: u32| call(uc, "Receive", &[from, to, flags, buf, size]).unwrap();
        assert_eq!(recv(&mut uc, 0), DPERR_NOMESSAGES);
        for ev in [
            Event::PlayerAdd { id: 5, name: "bob".into() },
            Event::Data { from: 5, to: 6, data: b"payload".to_vec() },
            Event::PlayerDel { id: 5 },
            Event::SessionLost,
        ] {
            dp(&mut uc).net.inject(ev);
        }
        let count = scratch(&mut uc, 4);
        call(&mut uc, "GetMessageCount", &[0, count]);
        assert_eq!(rd32(&uc, count), 4);
        // Buffer too small: the needed size is reported and the message stays queued.
        wr32(&mut uc, size, 4);
        assert_eq!(recv(&mut uc, 0), DPERR_BUFFERTOOSMALL);
        assert_eq!(rd32(&uc, size), 48 + 4);
        // DPMSG_CREATEPLAYERORGROUP, peeked first (flag 0x20), then consumed.
        wr32(&mut uc, size, 256);
        assert_eq!(recv(&mut uc, 0x20), DP_OK);
        assert_eq!((rd32(&uc, from), rd32(&uc, buf), rd32(&uc, buf + 8)), (0, 3, 5));
        assert_eq!((rd32(&uc, buf + 32), rd32(&uc, buf + 36)), (buf + 48, buf + 48), "string pointers are fixed up");
        assert_eq!(rd_cstr(&uc, buf + 48), "bob");
        wr32(&mut uc, size, 256);
        assert_eq!(recv(&mut uc, 0), DP_OK);
        call(&mut uc, "GetMessageCount", &[0, count]);
        assert_eq!(rd32(&uc, count), 3);
        // Application data keeps its sender and recipient.
        wr32(&mut uc, size, 256);
        assert_eq!(recv(&mut uc, 0), DP_OK);
        assert_eq!((rd32(&uc, from), rd32(&uc, to), rd32(&uc, size)), (5, 6, 7));
        assert_eq!(rd_bytes(&uc, buf, 7), b"payload");
        // DPMSG_DELETEPLAYERORGROUP then DPSYS_SESSIONLOST.
        wr32(&mut uc, size, 256);
        recv(&mut uc, 0);
        assert_eq!((rd32(&uc, buf), rd32(&uc, buf + 8), rd32(&uc, size)), (5, 5, 44));
        wr32(&mut uc, size, 256);
        recv(&mut uc, 0);
        assert_eq!(rd32(&uc, buf), 0x31);
        assert_eq!(recv(&mut uc, 0), DPERR_NOMESSAGES);
    }
}
