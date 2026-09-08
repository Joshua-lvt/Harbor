//! Happy-Eyeballs direct connects: race TCP across candidates, first
//! established socket wins.
//!
//! The caller orders candidates with IPv6 first (stable sort keeps invite
//! preference inside each family); every candidate starts connecting with a
//! small stagger so a fast IPv6 path wins ties without hammering every route
//! at once, and the fastest path still wins outright. Losers are detached
//! losers only: each racing thread is bounded by stagger plus its own dial
//! budget, then exits by itself holding nothing shared but a dead sender.
//! Threads plus std::net only, like the rest of the direct path.

use std::net::{SocketAddr, TcpStream};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Head start per candidate position: IPv6 (listed first) gets the first
/// shot, IPv4 follows before any single dial can time out.
pub const HE_STAGGER: Duration = Duration::from_millis(250);

/// Upper bound for a whole race: covers stagger across a full invite plus
/// one dial budget, with margin. Dialing never wedges the worker past this.
pub const RACE_OVERALL_TIMEOUT: Duration = Duration::from_secs(8);

/// Races `candidates` and returns the first established socket, or `None`
/// when every candidate refused, failed, or outlasted `overall`.
/// `per_attempt` bounds each single dial; `overall` bounds the race.
/// Empty input returns immediately without spawning anything.
pub fn race_connect(
    candidates: &[SocketAddr],
    per_attempt: Duration,
    overall: Duration,
) -> Option<TcpStream> {
    if candidates.is_empty() {
        return None;
    }
    let deadline = Instant::now() + overall;
    let (tx, rx) = mpsc::channel();
    for (index, addr) in candidates.iter().enumerate() {
        let tx = tx.clone();
        let addr = *addr;
        let delay = HE_STAGGER * index as u32;
        std::thread::spawn(move || {
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return;
            }
            if let Ok(socket) = TcpStream::connect_timeout(&addr, remaining.min(per_attempt)) {
                let _ = tx.send(socket);
            }
        });
    }
    drop(tx);
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return None;
    }
    rx.recv_timeout(remaining).ok()
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    fn closed_port() -> u16 {
        let probe = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        port
    }

    fn loopback_v4(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    #[test]
    fn empty_candidates_return_immediately() {
        let started = Instant::now();
        assert!(race_connect(&[], Duration::from_secs(3), Duration::from_secs(8)).is_none());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn all_refused_returns_none_inside_the_overall_budget() {
        let dead = vec![
            loopback_v4(closed_port()),
            loopback_v4(closed_port()),
            loopback_v4(closed_port()),
        ];
        let started = Instant::now();
        assert!(race_connect(&dead, Duration::from_secs(3), Duration::from_secs(5)).is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_live_listener_wins_over_refused_siblings() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let live = listener.local_addr().unwrap().port();
        let candidates = vec![loopback_v4(closed_port()), loopback_v4(live)];
        let winner = race_connect(&candidates, Duration::from_secs(3), Duration::from_secs(8))
            .expect("loopback listener answers");
        assert_eq!(winner.peer_addr().unwrap().port(), live);
    }

    #[test]
    fn ipv6_loopback_wins_when_ipv4_is_dead() {
        let listener = match TcpListener::bind("[::1]:0") {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("skipping IPv6 race test: cannot bind [::1] ({error})");
                return;
            }
        };
        let live = listener.local_addr().unwrap().port();
        let v6: SocketAddr = format!("[::1]:{live}").parse().unwrap();
        let candidates = vec![loopback_v4(closed_port()), v6];
        let winner = race_connect(&candidates, Duration::from_secs(3), Duration::from_secs(8))
            .expect("ipv6 loopback answers");
        assert!(winner.peer_addr().unwrap().is_ipv6());
    }
}
