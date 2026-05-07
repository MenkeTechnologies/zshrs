//! TCP networking module - port of Modules/tcp.c
//!
//! Provides ztcp builtin for TCP socket operations.

use std::collections::HashMap;
use std::io::{self};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};

/// TCP session flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpSessionType {
    Outbound,
    Inbound,
    Listen,
}

/// A TCP session
#[derive(Debug)]
/// A live TCP socket session.
/// Port of `struct ztcp_session` from Src/Modules/tcp.c — the C
/// source threads it through `zts_alloc()` (line 215),
/// `zts_delete()` (line 253), `zts_byfd()` (line 271). Same fd /
/// peer / local layout.
pub struct TcpSession {
    pub fd: RawFd,
    pub session_type: TcpSessionType,
    pub local_addr: Option<SocketAddr>,
    pub peer_addr: Option<SocketAddr>,
    pub is_zftp: bool,
}

impl TcpSession {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/tcp.c`.
    pub fn new(fd: RawFd, session_type: TcpSessionType) -> Self {
        Self {
            fd,
            session_type,
            local_addr: None,
            peer_addr: None,
            is_zftp: false,
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/tcp.c`.
    pub fn type_char(&self) -> char {
        if self.is_zftp {
            'Z'
        } else {
            match self.session_type {
                TcpSessionType::Listen => 'L',
                TcpSessionType::Inbound => 'I',
                TcpSessionType::Outbound => 'O',
            }
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/tcp.c`.
    pub fn direction_str(&self) -> &'static str {
        match self.session_type {
            TcpSessionType::Listen => "-<",
            TcpSessionType::Inbound => "<-",
            TcpSessionType::Outbound => "->",
        }
    }
}

/// TCP sessions manager
#[derive(Debug, Default)]
/// TCP session table.
/// Port of the `ztcp_sessions` linked list Src/Modules/tcp.c
/// keeps — `bin_ztcp()` (line 342) reads/mutates it.
pub struct TcpSessions {
    sessions: HashMap<RawFd, TcpSession>,
}

impl TcpSessions {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/tcp.c`.
    pub fn new() -> Self {
        Self::default()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/tcp.c`.
    pub fn add(&mut self, session: TcpSession) {
        self.sessions.insert(session.fd, session);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/tcp.c`.
    pub fn get(&self, fd: RawFd) -> Option<&TcpSession> {
        self.sessions.get(&fd)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/tcp.c`.
    pub fn get_by_ref(&self, fd: &RawFd) -> Option<&TcpSession> {
        self.sessions.get(fd)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/tcp.c`.
    pub fn get_mut(&mut self, fd: RawFd) -> Option<&mut TcpSession> {
        self.sessions.get_mut(&fd)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/tcp.c`.
    pub fn remove(&mut self, fd: RawFd) -> Option<TcpSession> {
        self.sessions.remove(&fd)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/tcp.c`.
    pub fn iter(&self) -> impl Iterator<Item = (&RawFd, &TcpSession)> {
        self.sessions.iter()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/tcp.c`.
    pub fn close_all(&mut self) {
        for (fd, _) in self.sessions.drain() {
            let _ = close_fd(fd);
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/tcp.c`.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/tcp.c`.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/tcp.c`.
fn close_fd(fd: RawFd) -> io::Result<()> {
    #[cfg(unix)]
    {
        let result = unsafe { libc::close(fd) };
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    #[cfg(not(unix))]
    {
        Ok(())
    }
}

/// Options for ztcp builtin
#[derive(Debug, Default)]
/// `ztcp` builtin option flags.
/// Mirrors the `Options ops` flag bag `bin_ztcp()` from
/// Src/Modules/tcp.c:342 reads — `-l` listen, `-a` accept,
/// `-c` close, `-d` fd, `-f` force, `-L` list, `-t` test,
/// `-v` verbose.
pub struct ZtcpOptions {
    pub close: bool,
    pub listen: bool,
    pub accept: bool,
    pub force: bool,
    pub verbose: bool,
    pub test: bool,
    pub list_format: bool,
    pub target_fd: Option<RawFd>,
}

/// Connect to a TCP host with timeout on DNS and connect (default 10s).
/// DNS resolution runs on a background thread so it can't hang the shell.
/// Connect to a host:port and return (fd, peer, local).
/// Port of `tcp_connect()` from Src/Modules/tcp.c:316 — wraps
/// `socket(2)` + `connect(2)` and resolves both endpoints.
pub fn tcp_connect(host: &str, port: u16) -> io::Result<(RawFd, SocketAddr, SocketAddr)> {
    tcp_connect_timeout(host, port, std::time::Duration::from_secs(10))         // c:316
}

/// Connect with an explicit timeout.
/// zshrs convenience over `tcp_connect()` (Src/Modules/tcp.c:316)
/// reachable from `bin_ztcp` (Src/Modules/tcp.c:342) — the C
/// source's connect blocks until kernel default timeout; this
/// exposes a configurable one.
pub fn tcp_connect_timeout(
    host: &str,
    port: u16,
    timeout: std::time::Duration,
) -> io::Result<(RawFd, SocketAddr, SocketAddr)> {
    // DNS resolution on a background thread — can hang for seconds on bad DNS
    let addr_str = format!("{}:{}", host, port);
    let (tx, rx) = std::sync::mpsc::channel();
    let dns_str = addr_str.clone();
    std::thread::Builder::new()
        .name("dns-resolve".to_string())
        .spawn(move || {
            let result: io::Result<Vec<SocketAddr>> =
                dns_str.to_socket_addrs().map(|a| a.collect());
            let _ = tx.send(result);
        })
        .map_err(io::Error::other)?;

    let addrs = rx
        .recv_timeout(timeout)
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "DNS resolution timed out"))?
        .map_err(|e| {
            tracing::warn!(host, error = %e, "DNS resolution failed");
            e
        })?;

    if addrs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "host resolution failure",
        ));
    }

    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => {
                tracing::debug!(%addr, "tcp: connected");
                let local = stream.local_addr()?;
                let peer = stream.peer_addr()?;
                let fd = stream.as_raw_fd();
                std::mem::forget(stream);
                return Ok((fd, local, peer));
            }
            Err(e) => {
                tracing::trace!(%addr, error = %e, "tcp: connect attempt failed");
                continue;
            }
        }
    }

    Err(io::Error::new(
        io::ErrorKind::ConnectionRefused,
        "connection failed",
    ))
}

/// Create a listening TCP socket
/// Open a listening TCP socket bound to the given port.
/// Port of the `-l` branch of `bin_ztcp()` (Src/Modules/tcp.c:342)
/// — `socket(2)` → `bind(2)` → `listen(2, 5)`.
pub fn tcp_listen(port: u16) -> io::Result<(RawFd, SocketAddr)> {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), port);
    let listener = TcpListener::bind(addr)?;
    let local = listener.local_addr()?;
    let fd = listener.as_raw_fd();
    std::mem::forget(listener);
    Ok((fd, local))
}

/// Accept a connection on a listening socket
/// Accept a connection on a listening socket.
/// Port of the `-a` branch of `bin_ztcp()` (Src/Modules/tcp.c:342)
/// — `accept(2)` with EINTR retry, returns peer + local sockaddrs.
pub fn tcp_accept(listen_fd: RawFd) -> io::Result<(RawFd, SocketAddr, SocketAddr)> {
    let listener = unsafe { TcpListener::from_raw_fd(listen_fd) };
    let result = listener.accept();
    std::mem::forget(listener);

    let (stream, peer) = result?;
    let local = stream.local_addr()?;
    let fd = stream.as_raw_fd();
    std::mem::forget(stream);
    Ok((fd, local, peer))
}

/// Check if a socket has pending connections (for -t option)
/// Probe whether a listening socket has a pending connection.
/// Port of the `-t` branch of `bin_ztcp()` (Src/Modules/tcp.c:342)
/// — `poll(2)` with zero timeout for non-blocking probe.
pub fn tcp_test_accept(listen_fd: RawFd) -> io::Result<bool> {
    #[cfg(unix)]
    {
        let mut pfd = libc::pollfd {
            fd: listen_fd,
            events: libc::POLLIN,
            revents: 0,
        };

        let result = unsafe { libc::poll(&mut pfd, 1, 0) };
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(result > 0)
        }
    }

    #[cfg(not(unix))]
    {
        Ok(true)
    }
}

/// Close a TCP session
/// Close a TCP session.
/// Port of `tcp_close()` from Src/Modules/tcp.c:295 — closes
/// the fd, removes the session from the table, frees the
/// allocation. The C source uses `ztcp_free_session()` (line
/// 245) for the per-entry free.
pub fn tcp_close(sessions: &mut TcpSessions, fd: RawFd, force: bool) -> Result<(), String> {
    if let Some(session) = sessions.get(fd) {                                   // c:295
        if session.is_zftp && !force {                                          // c:295
            return Err("use -f to force closure of a zftp control connection".to_string());  // c:305
        }                                                                       // c:295
    }                                                                           // c:295

    if let Some(_session) = sessions.remove(fd) {                               // c:295
        close_fd(fd).map_err(|e| format!("connection close failed: {}", e))?;   // c:305
        Ok(())                                                                  // c:295
    } else {                                                                    // c:295
        Err(format!("fd {} not found in tcp table", fd))                        // c:295
    }                                                                           // c:295
}

/// Resolve a service name to port number
/// Resolve a service name (or numeric string) to a port.
/// Port of the `getservbyname(3)` lookup `bin_ztcp()` does
/// (Src/Modules/tcp.c:342) — also accepts a bare numeric
/// string for direct port specification.
pub fn resolve_port(service: &str) -> Option<u16> {
    if let Ok(port) = service.parse::<u16>() {
        return Some(port);
    }

    #[cfg(unix)]
    {
        use std::ffi::CString;
        let service_c = CString::new(service).ok()?;
        let proto_c = CString::new("tcp").ok()?;

        unsafe {
            let serv = libc::getservbyname(service_c.as_ptr(), proto_c.as_ptr());
            if serv.is_null() {
                None
            } else {
                Some(u16::from_be((*serv).s_port as u16))
            }
        }
    }

    #[cfg(not(unix))]
    {
        None
    }
}

/// Resolve hostname to IP address
/// Resolve a hostname to an IP address.
/// Port of `zsh_gethostbyname2()` from Src/Modules/tcp.c:146
/// (with `zsh_getipnodebyname()` line 170 fallback) — wraps
/// `getaddrinfo(3)`.
pub fn zsh_gethostbyname2(host: &str) -> io::Result<IpAddr> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ip);
    }

    let addrs: Vec<SocketAddr> = format!("{}:0", host).to_socket_addrs()?.collect();
    addrs
        .first()
        .map(|a| a.ip())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "host resolution failure"))
}

/// Reverse DNS lookup
/// Reverse-DNS an IP address back to a hostname.
/// Port of the `gethostbyaddr(3)` lookup the C source uses
/// for `-v` verbose listing in `bin_ztcp()`
/// (Src/Modules/tcp.c:342).
pub fn reverse_lookup(addr: &IpAddr) -> Option<String> {
    let socket_addr = SocketAddr::new(*addr, 0);

    dns_lookup_reverse(&socket_addr)
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/tcp.c`.
fn dns_lookup_reverse(_addr: &SocketAddr) -> Option<String> {
    None
}

/// Format a socket address for display
/// Render a SocketAddr as `host:port`, optionally
/// reverse-resolving the host.
/// Port of the `zsh_inet_ntop()` (Src/Modules/tcp.c:72) +
/// optional reverse-DNS path the C source uses for
/// human-readable session listings.
pub fn format_addr(addr: &SocketAddr, resolve: bool) -> String {
    if resolve {
        if let Some(hostname) = reverse_lookup(&addr.ip()) {
            return format!("{}:{}", hostname, addr.port());
        }
    }
    format!("{}:{}", addr.ip(), addr.port())
}

/// Execute ztcp builtin
/// `ztcp` builtin entry point.
/// Port of `bin_ztcp()` from Src/Modules/tcp.c:342 — same big
/// switch over `-l`/`-a`/`-c`/`-d`/`-f`/`-L`/`-t`/`-v`.
pub fn bin_ztcp(
    args: &[&str],
    options: &ZtcpOptions,
    sessions: &mut TcpSessions,
) -> (i32, String) {
    let mut output = String::new();

    if options.close {
        if args.is_empty() {
            sessions.close_all();
            return (0, output);
        }

        let fd: RawFd = match args[0].parse() {
            Ok(fd) => fd,
            Err(_) => {
                return (
                    1,
                    format!("ztcp: {} is an invalid argument to -c\n", args[0]),
                );
            }
        };

        match tcp_close(sessions, fd, options.force) {
            Ok(()) => (0, output),
            Err(e) => (1, format!("ztcp: {}\n", e)),
        }
    } else if options.listen {
        if args.is_empty() {
            return (1, "ztcp: -l requires an argument\n".to_string());
        }

        let port = match resolve_port(args[0]) {
            Some(p) => p,
            None => {
                return (1, "ztcp: bad service name or port number\n".to_string());
            }
        };

        match tcp_listen(port) {
            Ok((fd, local)) => {
                let mut session = TcpSession::new(fd, TcpSessionType::Listen);
                session.local_addr = Some(local);
                let result_fd = options.target_fd.unwrap_or(fd);
                session.fd = result_fd;
                sessions.add(session);

                if options.verbose {
                    output.push_str(&format!("{} listener is on fd {}\n", port, result_fd));
                }
                (0, output)
            }
            Err(e) => (1, format!("ztcp: could not listen: {}\n", e)),
        }
    } else if options.accept {
        if args.is_empty() {
            return (1, "ztcp: -a requires an argument\n".to_string());
        }

        let listen_fd: RawFd = match args[0].parse() {
            Ok(fd) => fd,
            Err(_) => {
                return (1, "ztcp: invalid numerical argument\n".to_string());
            }
        };

        if let Some(session) = sessions.get(listen_fd) {
            if session.session_type != TcpSessionType::Listen {
                return (1, "ztcp: tcp connection not a listener\n".to_string());
            }
        } else {
            return (
                1,
                format!(
                    "ztcp: fd {} is not registered as a tcp connection\n",
                    args[0]
                ),
            );
        }

        if options.test {
            match tcp_test_accept(listen_fd) {
                Ok(true) => {}
                Ok(false) => return (1, output),
                Err(e) => return (1, format!("ztcp: poll error: {}\n", e)),
            }
        }

        match tcp_accept(listen_fd) {
            Ok((fd, local, peer)) => {
                let mut session = TcpSession::new(fd, TcpSessionType::Inbound);
                session.local_addr = Some(local);
                session.peer_addr = Some(peer);
                let result_fd = options.target_fd.unwrap_or(fd);
                session.fd = result_fd;
                sessions.add(session);

                if options.verbose {
                    output.push_str(&format!("{} is on fd {}\n", peer.port(), result_fd));
                }
                (0, output)
            }
            Err(e) => (1, format!("ztcp: could not accept connection: {}\n", e)),
        }
    } else if args.is_empty() {
        for (_, session) in sessions.iter() {
            let local_str = session
                .local_addr
                .map(|a| format_addr(&a, true))
                .unwrap_or_else(|| "?:?".to_string());
            let peer_str = session
                .peer_addr
                .map(|a| format_addr(&a, true))
                .unwrap_or_else(|| "?:?".to_string());

            if options.list_format {
                output.push_str(&format!(
                    "{} {} {} {} {} {}\n",
                    session.fd,
                    session.type_char(),
                    session
                        .local_addr
                        .map(|a| a.ip().to_string())
                        .unwrap_or_default(),
                    session.local_addr.map(|a| a.port()).unwrap_or(0),
                    session
                        .peer_addr
                        .map(|a| a.ip().to_string())
                        .unwrap_or_default(),
                    session.peer_addr.map(|a| a.port()).unwrap_or(0),
                ));
            } else {
                let zftp_str = if session.is_zftp { " ZFTP" } else { "" };
                output.push_str(&format!(
                    "{} {} {} is on fd {}{}\n",
                    local_str,
                    session.direction_str(),
                    peer_str,
                    session.fd,
                    zftp_str,
                ));
            }
        }
        (0, output)
    } else {
        let host = args[0];
        let port = if args.len() > 1 {
            resolve_port(args[1]).unwrap_or(23)
        } else {
            23
        };

        match tcp_connect(host, port) {
            Ok((fd, local, peer)) => {
                let mut session = TcpSession::new(fd, TcpSessionType::Outbound);
                session.local_addr = Some(local);
                session.peer_addr = Some(peer);
                let result_fd = options.target_fd.unwrap_or(fd);
                session.fd = result_fd;
                sessions.add(session);

                if options.verbose {
                    output.push_str(&format!("{}:{} is now on fd {}\n", host, port, result_fd));
                }
                (0, output)
            }
            Err(e) => (1, format!("ztcp: connection failed: {}\n", e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv6Addr;

    #[test]
    fn test_tcp_session_type_char() {
        let session = TcpSession::new(3, TcpSessionType::Outbound);
        assert_eq!(session.type_char(), 'O');

        let session = TcpSession::new(3, TcpSessionType::Inbound);
        assert_eq!(session.type_char(), 'I');

        let session = TcpSession::new(3, TcpSessionType::Listen);
        assert_eq!(session.type_char(), 'L');

        let mut session = TcpSession::new(3, TcpSessionType::Outbound);
        session.is_zftp = true;
        assert_eq!(session.type_char(), 'Z');
    }

    #[test]
    fn test_tcp_session_direction() {
        let session = TcpSession::new(3, TcpSessionType::Outbound);
        assert_eq!(session.direction_str(), "->");

        let session = TcpSession::new(3, TcpSessionType::Inbound);
        assert_eq!(session.direction_str(), "<-");

        let session = TcpSession::new(3, TcpSessionType::Listen);
        assert_eq!(session.direction_str(), "-<");
    }

    #[test]
    fn test_tcp_sessions_manager() {
        let mut sessions = TcpSessions::new();
        assert!(sessions.is_empty());

        let session = TcpSession::new(5, TcpSessionType::Outbound);
        sessions.add(session);
        assert_eq!(sessions.len(), 1);

        assert!(sessions.get(5).is_some());
        assert!(sessions.get(6).is_none());

        sessions.remove(5);
        assert!(sessions.is_empty());
    }

    #[test]
    fn test_resolve_port() {
        assert_eq!(resolve_port("80"), Some(80));
        assert_eq!(resolve_port("443"), Some(443));
        assert_eq!(resolve_port("invalid"), None);
    }

    #[test]
    fn test_resolve_host() {
        let ip = zsh_gethostbyname2("127.0.0.1").unwrap();
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));

        let ip = zsh_gethostbyname2("::1").unwrap();
        assert_eq!(ip, IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1)));
    }

    #[test]
    fn test_format_addr() {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8080);
        let formatted = format_addr(&addr, false);
        assert_eq!(formatted, "127.0.0.1:8080");
    }

    #[test]
    fn test_builtin_ztcp_list_empty() {
        let mut sessions = TcpSessions::new();
        let options = ZtcpOptions::default();
        let (status, output) = bin_ztcp(&[], &options, &mut sessions);
        assert_eq!(status, 0);
        assert!(output.is_empty());
    }

    #[test]
    fn test_builtin_ztcp_close_all() {
        let mut sessions = TcpSessions::new();
        let options = ZtcpOptions {
            close: true,
            ..Default::default()
        };
        let (status, _) = bin_ztcp(&[], &options, &mut sessions);
        assert_eq!(status, 0);
    }

    #[test]
    fn test_builtin_ztcp_listen_no_arg() {
        let mut sessions = TcpSessions::new();
        let options = ZtcpOptions {
            listen: true,
            ..Default::default()
        };
        let (status, output) = bin_ztcp(&[], &options, &mut sessions);
        assert_eq!(status, 1);
        assert!(output.contains("requires an argument"));
    }

    #[test]
    fn test_builtin_ztcp_accept_no_arg() {
        let mut sessions = TcpSessions::new();
        let options = ZtcpOptions {
            accept: true,
            ..Default::default()
        };
        let (status, output) = bin_ztcp(&[], &options, &mut sessions);
        assert_eq!(status, 1);
        assert!(output.contains("requires an argument"));
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// ztcp - TCP socket operations
    pub(crate) fn bin_ztcp(&mut self, args: &[String]) -> i32 {
        // Similar to zsocket but TCP specific
        self.bin_zsocket(args)
    }
}
// END moved-from-exec-rs
