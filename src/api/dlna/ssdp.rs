/*
 * Copyright (c) 2026 MirraNET, Niklas Linz. All rights reserved.
 *
 * This file is part of the MirraNET project and is licensed under the
 * GNU Lesser General Public License v3.0 (LGPLv3).
 *
 * You may use, distribute and modify this code under the terms
 * of the LGPLv3 license. You should have received a copy of the
 * license along with this file. If not, see <https://www.gnu.org/licenses/lgpl-3.0.html>
 * or contact: niklas.linz@mirranet.de
 */

//! SSDP, how players find the server: it answers their searches (`M-SEARCH`) on UDP port 1900
//! and announces itself to the multicast group at the start and every 15 minutes
//! (`NOTIFY ssdp:alive`), and says goodbye when it stops (`ssdp:byebye`). Only searches from the
//! home network are answered; the address in the answer is the server's own address on the
//! network card that reaches the player.

use super::xml::{CD, CM, DEVICE_TYPE};
use crate::media::playback::home_network;
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

pub const GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
pub const PORT: u16 = 1900;
const MAX_AGE: u32 = 1800;
const ANNOUNCE_EVERY: Duration = Duration::from_secs(15 * 60);

/// What the messages say about the server.
pub struct Info {
    /// `uuid:...`
    pub udn: String,
    /// HTTP port of the description and the media.
    pub port: u16,
    pub server: String,
    /// Changes with every start (UPnP 1.1 `BOOTID.UPNP.ORG`).
    pub boot_id: u32,
}

impl Info {
    /// Every notification type with its unique service name.
    fn targets(&self) -> Vec<(String, String)> {
        let u = &self.udn;
        vec![
            ("upnp:rootdevice".into(), format!("{u}::upnp:rootdevice")),
            (u.clone(), u.clone()),
            (DEVICE_TYPE.into(), format!("{u}::{DEVICE_TYPE}")),
            (CD.into(), format!("{u}::{CD}")),
            (CM.into(), format!("{u}::{CM}")),
        ]
    }

    fn location(&self, ip: Ipv4Addr) -> String {
        format!("http://{ip}:{}/dlna/description.xml", self.port)
    }

    /// The answers to a search target: all for `ssdp:all`, else the one that matches.
    pub fn answers(&self, st: &str) -> Vec<(String, String)> {
        self.targets().into_iter().filter(|(nt, _)| st == "ssdp:all" || nt.eq_ignore_ascii_case(st)).collect()
    }

    pub fn response(&self, ip: Ipv4Addr, st: &str, usn: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age={MAX_AGE}\r\nDATE: {}\r\nEXT:\r\nLOCATION: {}\r\nSERVER: {}\r\nST: {st}\r\nUSN: {usn}\r\nBOOTID.UPNP.ORG: {}\r\nCONFIGID.UPNP.ORG: 1\r\nCONTENT-LENGTH: 0\r\n\r\n",
            httpdate::fmt_http_date(std::time::SystemTime::now()),
            self.location(ip),
            self.server,
            self.boot_id,
        )
    }

    fn notify(&self, ip: Ipv4Addr, nts: &str, nt: &str, usn: &str) -> String {
        let location = if nts == "ssdp:alive" {
            format!("CACHE-CONTROL: max-age={MAX_AGE}\r\nLOCATION: {}\r\nSERVER: {}\r\n", self.location(ip), self.server)
        } else {
            String::new()
        };
        format!(
            "NOTIFY * HTTP/1.1\r\nHOST: {GROUP}:{PORT}\r\n{location}NT: {nt}\r\nNTS: {nts}\r\nUSN: {usn}\r\nBOOTID.UPNP.ORG: {}\r\nCONFIGID.UPNP.ORG: 1\r\n\r\n",
            self.boot_id
        )
    }
}

/// A search: its target and how long the player waits for answers (seconds).
#[derive(Debug, PartialEq)]
pub struct Search {
    pub st: String,
    pub mx: u64,
}

/// Reads an `M-SEARCH` request; None for anything else.
pub fn parse_search(packet: &str) -> Option<Search> {
    let mut lines = packet.split("\r\n").flat_map(|l| l.split('\n'));
    if !lines.next()?.trim().starts_with("M-SEARCH * HTTP/1.1") {
        return None;
    }
    let (mut st, mut man, mut mx) = (None, None, 1);
    for l in lines {
        let Some((k, v)) = l.split_once(':') else { continue };
        let v = v.trim().trim_matches('"').to_string();
        match k.trim().to_ascii_uppercase().as_str() {
            "ST" => st = Some(v),
            "MAN" => man = Some(v),
            "MX" => mx = v.parse().unwrap_or(1),
            _ => {}
        }
    }
    (man.as_deref() == Some("ssdp:discover")).then_some(Search { st: st?, mx })
}

/// The server's IPv4 address on the network card that reaches `peer`. No packet is sent.
pub fn local_ip_for(peer: IpAddr) -> Option<Ipv4Addr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect(SocketAddr::new(peer, PORT)).ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

/// The address announcements go out from: the card multicast is routed to.
fn announce_ip() -> Option<Ipv4Addr> {
    local_ip_for(IpAddr::V4(GROUP)).filter(|ip| !ip.is_loopback())
}

/// The socket on port 1900, shared with other UPnP programs on the same machine.
fn bind() -> std::io::Result<UdpSocket> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    #[cfg(all(unix, not(target_os = "solaris"), not(target_os = "illumos")))]
    s.set_reuse_port(true)?;
    s.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PORT).into())?;
    s.join_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED)?;
    if let Some(ip) = announce_ip() {
        // Also on the card multicast goes out from, if the default differs.
        let _ = s.join_multicast_v4(&GROUP, &ip);
    }
    s.set_nonblocking(true)?;
    Ok(s.into())
}

/// Sends `nts` announcements for every target, from `ip`.
fn announce(info: &Info, ip: Ipv4Addr, nts: &str) {
    let Ok(s) = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)) else { return };
    let _ = s.set_multicast_if_v4(&ip);
    let _ = s.set_multicast_ttl_v4(2);
    if s.bind(&SocketAddrV4::new(ip, 0).into()).is_err() {
        return;
    }
    let to: SocketAddr = SocketAddrV4::new(GROUP, PORT).into();
    for (nt, usn) in info.targets() {
        let _ = s.send_to(info.notify(ip, nts, &nt, &usn).as_bytes(), &to.into());
    }
}

/// Says goodbye, so players remove the server at once (when it stops or restarts).
pub fn goodbye(info: &Info) {
    if let Some(ip) = announce_ip() {
        announce(info, ip, "ssdp:byebye");
    }
}

/// Answers searches and announces the server until the runtime ends.
pub async fn run(info: Arc<Info>) -> std::io::Result<()> {
    let socket = Arc::new(tokio::net::UdpSocket::from_std(bind()?)?);
    let announcer = info.clone();
    tokio::spawn(async move {
        loop {
            // Three times at the start, as UDP may lose a packet.
            for _ in 0..3 {
                if let Some(ip) = announce_ip() {
                    announce(&announcer, ip, "ssdp:alive");
                }
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
            tokio::time::sleep(ANNOUNCE_EVERY).await;
        }
    });
    let mut buf = [0u8; 2048];
    loop {
        let Ok((n, from)) = socket.recv_from(&mut buf).await else { continue };
        if !home_network(&from.ip().to_string()) {
            tracing::debug!("DLNA: ignored a discovery packet from {from}, which is not in the home network");
            continue;
        }
        let Some(search) = parse_search(&String::from_utf8_lossy(&buf[..n])) else { continue };
        let answers = info.answers(&search.st);
        if answers.is_empty() {
            continue;
        }
        let Some(ip) = local_ip_for(from.ip()) else { continue };
        tracing::debug!("DLNA search from {from} for {}", search.st);
        let (socket, info) = (socket.clone(), info.clone());
        tokio::spawn(async move {
            // Spread the answers within the time the player waits (at most a second).
            let wait = rand::Rng::gen_range(&mut rand::thread_rng(), 0..=search.mx.clamp(1, 5) * 200);
            tokio::time::sleep(Duration::from_millis(wait)).await;
            for (st, usn) in answers {
                let _ = socket.send_to(info.response(ip, &st, &usn).as_bytes(), from).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> Info {
        Info { udn: "uuid:abc".into(), port: 32501, server: "Linux/1.0 UPnP/1.0 FreeX/0.21.0".into(), boot_id: 7 }
    }

    #[test]
    fn searches() {
        let m = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 3\r\nST: urn:schemas-upnp-org:device:MediaServer:1\r\n\r\n";
        assert_eq!(parse_search(m), Some(Search { st: DEVICE_TYPE.into(), mx: 3 }));
        assert_eq!(parse_search("NOTIFY * HTTP/1.1\r\nNT: x\r\n\r\n"), None);
        assert_eq!(parse_search("M-SEARCH * HTTP/1.1\r\nST: ssdp:all\r\n\r\n"), None, "MAN is required");
        let i = info();
        assert_eq!(i.answers("ssdp:all").len(), 5);
        assert_eq!(i.answers("upnp:rootdevice"), vec![("upnp:rootdevice".to_string(), "uuid:abc::upnp:rootdevice".to_string())]);
        assert!(i.answers("urn:schemas-upnp-org:device:MediaRenderer:1").is_empty());
        let r = i.response(Ipv4Addr::new(192, 168, 1, 5), "upnp:rootdevice", "uuid:abc::upnp:rootdevice");
        assert!(r.starts_with("HTTP/1.1 200 OK\r\n") && r.contains("LOCATION: http://192.168.1.5:32501/dlna/description.xml\r\n"));
        assert!(r.ends_with("\r\n\r\n"));
    }
}
