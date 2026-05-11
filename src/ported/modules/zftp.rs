//! ZFTP module - port of Modules/zftp.c
//!
//! it's a TELNET based protocol, but don't think I like doing this         // c:56
//! Number of connections actually open                                      // c:210
//! zfclosing is set if zftp_close() is active                               // c:219
//! List of active sessions                                                  // c:310
//!
//! Provides a builtin FTP client for zsh.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

// `TransferType` enum removed — was Rust-only invention. C uses the
// `ZFST_ASCI` (0x0000) / `ZFST_IMAG` (0x0001) bits from the ZFST_*
// status word (c:246-247) for the next-transfer type, and
// `ZFST_CASC` (0x0000) / `ZFST_CIMA` (0x0002) for the current-transfer
// type. Callers store the type as `i32` and compare via the `ZFST_TYPE`
// macro (c:267): `ZFST_TYPE(x) (x & ZFST_TMSK)`.
//
// Inline-test pattern matching C `if (zfst_status & ZFST_IMAG) ...`
// at every TYPE-letter dispatch (e.g. zftp.c around `zftp_type` body):
//   `if (typ & ZFST_IMAG) != 0 { "I" } else { "A" }`

// `TransferMode` enum removed — was Rust-only invention. C uses the
// `ZFST_STRE` (0x0000) / `ZFST_BLOC` (0x0004) bits from the ZFST_*
// status word (defined later in this file at the c:245 enum). Callers
// store the mode as `i32` and compare against those constants directly:
//   `if (mode & ZFST_BLOC) != 0 { "B" } else { "S" }`
// — same inline-test pattern the C source uses at every MODE send-site.

/// FTP server response (3-digit code + message).
/// Port of the response handling inside `zfgetmsg()` from
/// `lastmsg` — file-scope global from `Src/Modules/zftp.c:227`:
/// `static char *lastmsg, lastcodestr[4];`. Holds the most recent
/// FTP server reply message text (post-3-digit-code body).
pub static lastmsg: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// `lastcodestr` — file-scope global from `Src/Modules/zftp.c:227`.
/// 3-digit FTP reply code as ASCII (`"000".."599"`), zero-terminated
/// to 4 bytes in C; mirrored as a 4-byte Mutex array for parity.
pub static lastcodestr: std::sync::Mutex<[u8; 4]>
    = std::sync::Mutex::new([b'0', b'0', b'0', 0]);

/// `lastcode` — file-scope global from `Src/Modules/zftp.c:228`:
/// `static int lastcode;`. Numeric form of `lastcodestr`.
pub static lastcode: std::sync::atomic::AtomicI32
    = std::sync::atomic::AtomicI32::new(0);

// `FtpResponse` struct removed — was Rust-only invention. C source
// returns plain `int` from every reply-handling fn and reads the
// `lastcode` / `lastmsg` globals (c:227-228) inline at each check
// site. Callers in this Rust port use the same pattern: each fn
// returns `i32` (matching C `int`), and `lastmsg.lock().unwrap()`
// + `lastcode.load(Relaxed)` provide the C-equivalent inline reads.
//
// For ergonomic call sites the type alias below carries both halves
// without inventing a new struct shape:
#[allow(non_camel_case_types)]
pub type FtpResponse = (i32, String);

// =====================================================================
// `struct zfheader` from `Src/Modules/zftp.c:114` — block-mode header.
// =====================================================================

/// Port of `struct zfheader` from `Src/Modules/zftp.c:114`.
/// ```c
/// struct zfheader {
///     char flags;
///     unsigned char bytes[2];
/// };
/// ```
#[allow(non_camel_case_types)]
pub struct zfheader {
    pub flags: i8,                                                       // c:115
    pub bytes: [u8; 2],                                                   // c:116
}

// =====================================================================
// `enum { ZFHD_* }` from `Src/Modules/zftp.c:119` — block-header flags.
// =====================================================================

/// `ZFHD_MARK` — restart marker.
pub const ZFHD_MARK: i32 = 16;                                            // c:120
/// `ZFHD_ERRS` — suspected errors in block.
pub const ZFHD_ERRS: i32 = 32;                                            // c:121
/// `ZFHD_EOFB` — block is end of record.
pub const ZFHD_EOFB: i32 = 64;                                            // c:122
/// `ZFHD_EORB` — block is end of file.
pub const ZFHD_EORB: i32 = 128;                                           // c:123

/// `readwrite_t` — function pointer typedef from
/// `Src/Modules/zftp.c:126`: `typedef int (*readwrite_t)(int, char *, off_t, int);`
#[allow(non_camel_case_types)]
pub type readwrite_t = fn(i32, &mut [u8], libc::off_t, i32) -> i32;

// =====================================================================
// `struct zftpcmd` from `Src/Modules/zftp.c:128` — subcommand entry.
// =====================================================================

/// Port of `struct zftpcmd` from `Src/Modules/zftp.c:128`.
/// ```c
/// struct zftpcmd {
///     const char *nam;
///     int (*fun) (char *, char **, int);
///     int min, max, flags;
/// };
/// ```
#[allow(non_camel_case_types)]
pub struct zftpcmd {
    pub nam: &'static str,                                               // c:129
    pub fun: fn(&str, &[&str], i32) -> i32,                              // c:130
    pub min: i32,                                                         // c:131
    pub max: i32,
    pub flags: i32,
}

/// Port of `typedef struct zftpcmd *Zftpcmd` from `Src/Modules/zftp.c:151`.
#[allow(non_camel_case_types)]
pub type Zftpcmd = Box<zftpcmd>;

// =====================================================================
// `enum { ZFTP_* }` from `Src/Modules/zftp.c:134` — zftpcmd.flags bits.
// =====================================================================

/// `ZFTP_CONN` — must be connected.
pub const ZFTP_CONN: i32 = 0x0001;                                        // c:135
/// `ZFTP_LOGI` — must be logged in.
pub const ZFTP_LOGI: i32 = 0x0002;                                        // c:136
/// `ZFTP_TBIN` — set transfer type image.
pub const ZFTP_TBIN: i32 = 0x0004;                                        // c:137
/// `ZFTP_TASC` — set transfer type ASCII.
pub const ZFTP_TASC: i32 = 0x0008;                                        // c:138
/// `ZFTP_NLST` — use NLST rather than LIST.
pub const ZFTP_NLST: i32 = 0x0010;                                        // c:139
/// `ZFTP_DELE` — a delete rather than a make.
pub const ZFTP_DELE: i32 = 0x0020;                                        // c:140
/// `ZFTP_SITE` — a site rather than a quote.
pub const ZFTP_SITE: i32 = 0x0040;                                        // c:141
/// `ZFTP_APPE` — append rather than overwrite.
pub const ZFTP_APPE: i32 = 0x0080;                                        // c:142
/// `ZFTP_HERE` — here rather than over there.
pub const ZFTP_HERE: i32 = 0x0100;                                        // c:143
/// `ZFTP_CDUP` — CDUP rather than CWD.
pub const ZFTP_CDUP: i32 = 0x0200;                                        // c:144
/// `ZFTP_REST` — restart: set point in remote file.
pub const ZFTP_REST: i32 = 0x0400;                                        // c:145
/// `ZFTP_RECV` — receive rather than send.
pub const ZFTP_RECV: i32 = 0x0800;                                        // c:146
/// `ZFTP_TEST` — test command, don't test.
pub const ZFTP_TEST: i32 = 0x1000;                                        // c:147
/// `ZFTP_SESS` — session command, don't need status.
pub const ZFTP_SESS: i32 = 0x2000;                                        // c:148

/// `static char *zfparams[]` from `Src/Modules/zftp.c:197` — list of
/// non-special params to unset when a connection closes.
pub static ZFPARAMS: &[&str] = &[
    "ZFTP_HOST", "ZFTP_PORT", "ZFTP_IP", "ZFTP_SYSTEM", "ZFTP_USER",
    "ZFTP_ACCOUNT", "ZFTP_PWD", "ZFTP_TYPE", "ZFTP_MODE",                // c:198-199
];

// =====================================================================
// `enum { ZFPM_* }` from `Src/Modules/zftp.c:204` — zfsetparam flags.
// =====================================================================

/// `ZFPM_READONLY` — make parameter readonly.
pub const ZFPM_READONLY: i32 = 0x01;                                      // c:205
/// `ZFPM_IFUNSET` — only set if not already set.
pub const ZFPM_IFUNSET: i32 = 0x02;                                       // c:206
/// `ZFPM_INTEGER` — passed pointer to off_t.
pub const ZFPM_INTEGER: i32 = 0x04;                                       // c:207

/// `zfnopen` — file-scope global from `Src/Modules/zftp.c:211`:
/// `static int zfnopen;` — number of connections actually open.
pub static ZFNOPEN: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// `zcfinish` — file-scope global from `Src/Modules/zftp.c:218`:
/// `static int zcfinish;` — 0 keep going, 1 line finished, 2 EOF.
pub static ZCFINISH: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// `zfclosing` — file-scope global from `Src/Modules/zftp.c:220`:
/// `static int zfclosing;` — set when zftp_close() is active.
pub static ZFCLOSING: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

// =====================================================================
// `enum { ZFCP_* }` from `Src/Modules/zftp.c` — server-capability
// tri-state for SIZE / MDTM probes.
// =====================================================================

/// `ZFCP_UNKN` — dunno if it works on this server. Port of c:`enum`
/// in `Src/Modules/zftp.c`.
pub const ZFCP_UNKN: i32 = 0;
/// `ZFCP_YUPP` — server supports the feature.
pub const ZFCP_YUPP: i32 = 1;
/// `ZFCP_NOPE` — server doesn't support the feature.
pub const ZFCP_NOPE: i32 = 2;

// =====================================================================
// `enum { ZFST_* }` from `Src/Modules/zftp.c` — bit-packed shared-fd
// status word used by the `zfstatfd` mechanism so a subshell can
// detect type/mode/connection changes in the parent shell.
// =====================================================================

/// `ZF_BUFSIZE` from `Src/Modules/zftp.c:1458`. Default I/O block
/// size for the zftp byte-stream pump.
pub const ZF_BUFSIZE: usize = 32_768;                                        // c:1458

/// `ZF_ASCSIZE` from `Src/Modules/zftp.c:1459`.
/// `#define ZF_ASCSIZE (ZF_BUFSIZE/2)`. Smaller buffer for ASCII
/// mode (line-by-line CRLF translation can grow output up to 2x).
pub const ZF_ASCSIZE: usize = ZF_BUFSIZE / 2;                                // c:1459

/// `ZFST_ASCI` — type for next transfer is ASCII.
pub const ZFST_ASCI: i32 = 0x0000;
/// `ZFST_IMAG` — type for next transfer is image (binary).
pub const ZFST_IMAG: i32 = 0x0001;
/// `ZFST_TMSK` — mask for type flags.
pub const ZFST_TMSK: i32 = 0x0001;
/// `ZFST_TBIT` — number of bits in type flags.
pub const ZFST_TBIT: i32 = 0x0001;
/// `ZFST_CASC` — current type is ASCII (default).
pub const ZFST_CASC: i32 = 0x0000;
/// `ZFST_CIMA` — current type is image.
pub const ZFST_CIMA: i32 = 0x0002;
/// `ZFST_STRE` — stream mode (default).
pub const ZFST_STRE: i32 = 0x0000;
/// `ZFST_BLOC` — block mode.
pub const ZFST_BLOC: i32 = 0x0004;
/// `ZFST_MMSK` — mask for mode flags.
pub const ZFST_MMSK: i32 = 0x0004;
/// `ZFST_LOGI` — user logged in.
pub const ZFST_LOGI: i32 = 0x0008;
/// `ZFST_SYST` — done SYST type check.
pub const ZFST_SYST: i32 = 0x0010;
/// `ZFST_NOPS` — server doesn't understand PASV.
pub const ZFST_NOPS: i32 = 0x0020;
/// `ZFST_NOSZ` — server doesn't send `(XXXX bytes)' reply.
pub const ZFST_NOSZ: i32 = 0x0040;
/// `ZFST_TRSZ` — tried getting 'size' from reply.
pub const ZFST_TRSZ: i32 = 0x0080;
/// `ZFST_CLOS` — connection closed.
pub const ZFST_CLOS: i32 = 0x0100;

/// `ZFST_TYPE(x)` macro — extract type-flag bits.
/// Port of `#define ZFST_TYPE(x) (x & ZFST_TMSK)` from
/// `Src/Modules/zftp.c`.
#[allow(non_snake_case)]
#[inline]
pub fn ZFST_TYPE(x: i32) -> i32 { x & ZFST_TMSK }

/// `ZFST_MODE(x)` macro — extract mode-flag bits.
/// Port of `#define ZFST_MODE(x) (x & ZFST_MMSK)` from
/// `Src/Modules/zftp.c`.
#[allow(non_snake_case)]
#[inline]
pub fn ZFST_MODE(x: i32) -> i32 { x & ZFST_MMSK }

/// Port of `struct zftp_session` from `Src/Modules/zftp.c:299`.
///
/// C definition (verbatim):
/// ```c
/// struct zftp_session {
///     char *name;            /* name of session */
///     char **params;         /* parameters ordered as in zfparams */
///     char **userparams;     /* user parameters set by zftp_params */
///     FILE *cin;             /* control input file */
///     Tcp_session control;   /* the control connection */
///     int dfd;               /* data connection */
///     int has_size;          /* understands SIZE? */
///     int has_mdtm;          /* understands MDTM? */
/// };
///
/// typedef struct zftp_session *Zftp_session;  // c:50
/// ```
///
/// Field names + order match C exactly. `cin` (control input file) is
/// modelled as `Option<TcpStream>` since Rust doesn't expose libc
/// FILE* directly; `control` (the Tcp_session) collapses into the
/// same TcpStream slot in the static-link path.
#[derive(Debug)]
#[allow(non_camel_case_types)]
pub struct zftp_session {
    pub name: String,                                                    // c:300 char *name
    pub params: Vec<String>,                                              // c:301 char **params
    pub userparams: Vec<String>,                                          // c:302 char **userparams
    pub cin: Option<TcpStream>,                                          // c:303 FILE *cin (control input)
    pub control: Option<TcpStream>,                                       // c:304 Tcp_session control
    pub dfd: i32,                                                         // c:305 int dfd
    pub has_size: i32,                                                    // c:306 int has_size
    pub has_mdtm: i32,                                                    // c:307 int has_mdtm

    // Below: ergonomic Rust fields not in C `struct zftp_session` but
    // needed by the Rust wrapper to track connection state without the
    // C `params` array indexing convention. Document the mapping back
    // to C `params[]` slots in comments.
    pub host: Option<String>,            // C: params[ZFPM_HOST]
    pub port: u16,                       // C: params[ZFPM_PORT] (parsed)
    pub user: Option<String>,            // C: params[ZFPM_USER]
    pub pwd: Option<String>,             // C: params[ZFPM_PASSWORD]
    pub connected: bool,                 // C: cin != NULL
    pub logged_in: bool,                 // C: derived from greeting parse
    pub transfer_type: i32,
    pub transfer_mode: i32,
    pub passive: bool,
}

/// Port of `typedef struct zftp_session *Zftp_session;` from
/// `Src/Modules/zftp.c:50`. Pointer-style typedef alias used by every
/// `zftp_*` callsite that takes a session arg.
#[allow(non_camel_case_types)]
pub type Zftp_session = Box<zftp_session>;

impl zftp_session {
    /// Port of `newsession()` from `Src/Modules/zftp.c`. C uses
    /// `zshcalloc(sizeof(struct zftp_session))` then sets `name`
    /// (c:2891 `zfsess->name = ztrdup(name);`) and pre-allocates the
    /// `params` / `userparams` arrays. Same default state.
    pub fn new(name: &str) -> Self {
        Self {
            // C-faithful fields from `struct zftp_session` (c:299):
            name: name.to_string(),                                       // c:300
            params: Vec::new(),                                           // c:301
            userparams: Vec::new(),                                       // c:302
            cin: None,                                                    // c:303 NULL
            control: None,                                                // c:304 NULL
            dfd: -1,                                                      // c:305 (closed)
            has_size: 0,                                                  // c:306
            has_mdtm: 0,                                                  // c:307
            // Ergonomic Rust-side state mirroring C's params[] indices:
            host: None,
            port: 21,
            user: None,
            pwd: None,
            connected: false,
            logged_in: false,
            transfer_type: ZFST_IMAG,
            transfer_mode: ZFST_STRE,
            passive: true,
        }
    }

    fn send_command(&mut self, cmd: &str) -> io::Result<()> {
        if let Some(ref mut stream) = self.cin {
            write!(stream, "{}\r\n", cmd)?;
            stream.flush()
        } else {
            Err(io::Error::new(io::ErrorKind::NotConnected, "not connected"))
        }
    }

    /// Port of `zfgetmsg()` from `Src/Modules/zftp.c:702`.
    fn read_response(&mut self) -> io::Result<FtpResponse> {
        let stream = self
            .cin
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "not connected"))?;

        let mut reader = BufReader::new(stream.try_clone()?);
        let mut full_message = String::new();
        let mut code = 0u32;
        let mut multiline = false;
        let mut first_code = String::new();

        loop {
            let mut line = String::new();
            reader.read_line(&mut line)?;
            let line = line.trim_end();

            if line.len() < 3 {
                continue;
            }

            if code == 0 {
                first_code = line[..3].to_string();
                code = first_code.parse().unwrap_or(0);

                if line.len() > 3 && line.chars().nth(3) == Some('-') {
                    multiline = true;
                }
            }

            full_message.push_str(line);
            full_message.push('\n');

            if multiline {
                if line.starts_with(&first_code)
                    && line.len() > 3
                    && line.chars().nth(3) == Some(' ')
                {
                    break;
                }
            } else {
                break;
            }
        }

        Ok((code as i32, full_message))
    }

    /// Port of `zftp_open()` from `Src/Modules/zftp.c:1690`.
    /// Connect to FTP server — DNS resolution on background thread to avoid hangs
    pub fn connect(&mut self, host: &str, port: Option<u16>) -> io::Result<FtpResponse> {
        let port = port.unwrap_or(21);
        let addr_str = format!("{}:{}", host, port);
        let dns_timeout = Duration::from_secs(10);

        // DNS on background thread
        let (tx, rx) = std::sync::mpsc::channel();
        let dns = addr_str.clone();
        std::thread::Builder::new()
            .name("zftp-dns".to_string())
            .spawn(move || {
                let _ = tx.send(dns.to_socket_addrs().map(|a| a.collect::<Vec<_>>()));
            })
            .map_err(io::Error::other)?;

        let addrs = rx
            .recv_timeout(dns_timeout)
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "DNS resolution timed out"))?
            .map_err(|e| {
                tracing::warn!(host, error = %e, "zftp: DNS failed");
                e
            })?;

        let sock_addr = addrs
            .into_iter()
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid address"))?;

        let stream = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(30))?;

        stream.set_read_timeout(Some(Duration::from_secs(60)))?;
        stream.set_write_timeout(Some(Duration::from_secs(60)))?;

        self.cin = Some(stream);
        self.host = Some(host.to_string());
        self.port = port;
        self.connected = true;

        self.read_response()
    }

    /// Port of `zftp_login()` from `Src/Modules/zftp.c:2118`.
    /// Login to FTP server
    pub fn login(&mut self, user: &str, pass: Option<&str>) -> io::Result<FtpResponse> {
        self.send_command(&format!("USER {}", user))?;
        let resp = self.read_response()?;

        if resp.0 == 331 {
            let password = pass.unwrap_or("");
            self.send_command(&format!("PASS {}", password))?;
            let resp = self.read_response()?;

            if (resp.0 >= 200 && resp.0 < 300) {
                self.logged_in = true;
                self.user = Some(user.to_string());
            }
            return Ok(resp);
        }

        if (resp.0 >= 200 && resp.0 < 300) {
            self.logged_in = true;
            self.user = Some(user.to_string());
        }

        Ok(resp)
    }

    /// Set transfer type
    pub fn set_type(&mut self, transfer_type: i32) -> io::Result<FtpResponse> {
        // C inline pattern: `(typ & ZFST_IMAG) ? "I" : "A"`
        let typ_letter = if (transfer_type & ZFST_IMAG) != 0 { "I" } else { "A" };
        self.send_command(&format!("TYPE {}", typ_letter))?;
        let resp = self.read_response()?;
        if (resp.0 >= 200 && resp.0 < 300) {
            self.transfer_type = transfer_type;
        }
        Ok(resp)
    }

    /// Change directory
    pub fn cd(&mut self, path: &str) -> io::Result<FtpResponse> {
        self.send_command(&format!("CWD {}", path))?;
        self.read_response()
    }

    /// Change to parent directory
    pub fn cdup(&mut self) -> io::Result<FtpResponse> {
        self.send_command("CDUP")?;
        self.read_response()
    }

    /// Get current directory
    pub fn pwd(&mut self) -> io::Result<(FtpResponse, Option<String>)> {
        self.send_command("PWD")?;
        let resp = self.read_response()?;

        let pwd = if (resp.0 >= 200 && resp.0 < 300) {
            if let Some(start) = resp.1.find('"') {
                if let Some(end) = resp.1[start + 1..].find('"') {
                    Some(resp.1[start + 1..start + 1 + end].to_string())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        Ok((resp, pwd))
    }

    /// List directory
    pub fn list(&mut self, path: Option<&str>) -> io::Result<(FtpResponse, Vec<String>)> {
        let data_stream = self.enter_passive_mode()?;

        let cmd = match path {
            Some(p) => format!("LIST {}", p),
            None => "LIST".to_string(),
        };
        self.send_command(&cmd)?;
        let resp = self.read_response()?;

        if !(resp.0 >= 100 && resp.0 < 400) {
            return Ok((resp, Vec::new()));
        }

        let mut reader = BufReader::new(data_stream);
        let mut lines = Vec::new();
        let mut line = String::new();
        while reader.read_line(&mut line)? > 0 {
            lines.push(line.trim_end().to_string());
            line.clear();
        }

        let final_resp = self.read_response()?;

        Ok((final_resp, lines))
    }

    /// List filenames only
    pub fn nlst(&mut self, path: Option<&str>) -> io::Result<(FtpResponse, Vec<String>)> {
        let data_stream = self.enter_passive_mode()?;

        let cmd = match path {
            Some(p) => format!("NLST {}", p),
            None => "NLST".to_string(),
        };
        self.send_command(&cmd)?;
        let resp = self.read_response()?;

        if !(resp.0 >= 100 && resp.0 < 400) {
            return Ok((resp, Vec::new()));
        }

        let mut reader = BufReader::new(data_stream);
        let mut lines = Vec::new();
        let mut line = String::new();
        while reader.read_line(&mut line)? > 0 {
            lines.push(line.trim_end().to_string());
            line.clear();
        }

        let final_resp = self.read_response()?;

        Ok((final_resp, lines))
    }

    /// Port of `zftp_open()` from `Src/Modules/zftp.c:1690`.
    fn enter_passive_mode(&mut self) -> io::Result<TcpStream> {
        self.send_command("PASV")?;
        let resp = self.read_response()?;

        if !(resp.0 >= 200 && resp.0 < 300) {
            return Err(io::Error::other(resp.1));
        }

        let (ip, port) = zfopendata(&resp.1)?;
        let addr = format!("{}:{}", ip, port);

        TcpStream::connect_timeout(
            &addr.to_socket_addrs()?.next().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "invalid PASV address")
            })?,
            Duration::from_secs(30),
        )
    }

    /// Port of `zfstats()` from `Src/Modules/zftp.c:1193`.
    /// Download a file
    pub fn get(&mut self, remote: &str, local: &Path) -> io::Result<FtpResponse> {
        let mut data_stream = self.enter_passive_mode()?;

        self.send_command(&format!("RETR {}", remote))?;
        let resp = self.read_response()?;

        if !(resp.0 >= 100 && resp.0 < 400) {
            return Ok(resp);
        }

        let mut file = std::fs::File::create(local)?;
        let mut buf = [0u8; 8192];
        loop {
            let n = data_stream.read(&mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
        }

        self.read_response()
    }

    /// Port of `zfstats()` from `Src/Modules/zftp.c:1193`.
    /// Upload a file
    pub fn put(&mut self, local: &Path, remote: &str) -> io::Result<FtpResponse> {
        let mut data_stream = self.enter_passive_mode()?;

        self.send_command(&format!("STOR {}", remote))?;
        let resp = self.read_response()?;

        if !(resp.0 >= 100 && resp.0 < 400) {
            return Ok(resp);
        }

        let mut file = std::fs::File::open(local)?;
        let mut buf = [0u8; 8192];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            data_stream.write_all(&buf[..n])?;
        }
        drop(data_stream);

        self.read_response()
    }

    /// Delete a file
    pub fn delete(&mut self, path: &str) -> io::Result<FtpResponse> {
        self.send_command(&format!("DELE {}", path))?;
        self.read_response()
    }

    /// Make directory
    pub fn mkdir(&mut self, path: &str) -> io::Result<FtpResponse> {
        self.send_command(&format!("MKD {}", path))?;
        self.read_response()
    }

    /// Remove directory
    pub fn rmdir(&mut self, path: &str) -> io::Result<FtpResponse> {
        self.send_command(&format!("RMD {}", path))?;
        self.read_response()
    }

    /// Rename file
    pub fn rename(&mut self, from: &str, to: &str) -> io::Result<FtpResponse> {
        self.send_command(&format!("RNFR {}", from))?;
        let resp = self.read_response()?;

        if !(resp.0 >= 300 && resp.0 < 400) {
            return Ok(resp);
        }

        self.send_command(&format!("RNTO {}", to))?;
        self.read_response()
    }

    /// Get file size
    pub fn size(&mut self, path: &str) -> io::Result<(FtpResponse, Option<u64>)> {
        self.send_command(&format!("SIZE {}", path))?;
        let resp = self.read_response()?;

        let size = if (resp.0 >= 200 && resp.0 < 300) {
            resp.1
                .split_whitespace()
                .last()
                .and_then(|s| s.parse().ok())
        } else {
            None
        };

        Ok((resp, size))
    }

    /// Send raw command
    pub fn bslashquote(&mut self, cmd: &str) -> io::Result<FtpResponse> {
        self.send_command(cmd)?;
        self.read_response()
    }

    /// Port of `zftp_open()` from `Src/Modules/zftp.c:1690`.
    /// Close connection
    pub fn close(&mut self) -> io::Result<FtpResponse> {
        if !self.connected {
            return Ok((0, "not connected".to_string()));
        }

        let resp = if let Ok(()) = self.send_command("QUIT") {
            self.read_response().unwrap_or_else(|_| (221, "Goodbye".to_string()))
        } else {
            (221, "Goodbye".to_string())
        };

        self.cin = None;
        self.connected = false;
        self.logged_in = false;
        self.host = None;
        self.user = None;
        self.pwd = None;

        Ok(resp)
    }
}

/// Port of `zfopendata()` from `Src/Modules/zftp.c:859`.
fn zfopendata(msg: &str) -> io::Result<(String, u16)> {
    let start = msg
        .find('(')
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid PASV response"))?;
    let end = msg
        .find(')')
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid PASV response"))?;

    let nums: Vec<u16> = msg[start + 1..end]
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    if nums.len() != 6 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid PASV numbers",
        ));
    }

    let ip = format!("{}.{}.{}.{}", nums[0], nums[1], nums[2], nums[3]);
    let port = (nums[4] << 8) + nums[5];

    Ok((ip, port))
}

/// FTP sessions manager.
/// Port of the file-static `zfsess_node` linked list +
/// `zfsess_current` pointer Src/Modules/zftp.c keeps —
/// `zftp_session()` (line 2889) drives the switch,
/// `zftp_rmsession()` (line 2915) the removal.
#[derive(Debug, Default)]
pub struct Zftp {
    sessions: HashMap<String, zftp_session>,
    current: Option<String>,
}

impl Zftp {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_session(&self, name: Option<&str>) -> Option<&zftp_session> {
        let key = name
            .map(|s| s.to_string())
            .or_else(|| self.current.clone())?;
        self.sessions.get(&key)
    }

    pub fn get_session_mut(&mut self, name: Option<&str>) -> Option<&mut zftp_session> {
        let key = name
            .map(|s| s.to_string())
            .or_else(|| self.current.clone())?;
        self.sessions.get_mut(&key)
    }

    pub fn create_session(&mut self, name: &str) -> &mut zftp_session {
        self.sessions
            .entry(name.to_string())
            .or_insert_with(|| zftp_session::new(name))
    }

    /// Port of `zftp_rmsession()` from `Src/Modules/zftp.c:2915`.
    pub fn remove_session(&mut self, name: &str) -> Option<zftp_session> {
        let sess = self.sessions.remove(name);
        if self.current.as_deref() == Some(name) {
            // After dropping the current session, pick the
            // alphabetically-first remaining session (deterministic;
            // HashMap::keys().next() picks at random).
            let mut keys: Vec<&String> = self.sessions.keys().collect();
            keys.sort();
            self.current = keys.first().map(|s| (*s).clone());
        }
        sess
    }

    pub fn set_current(&mut self, name: &str) -> bool {
        if self.sessions.contains_key(name) {
            self.current = Some(name.to_string());
            true
        } else {
            false
        }
    }

    pub fn current_name(&self) -> Option<&str> {
        self.current.as_deref()
    }

    pub fn session_names(&self) -> Vec<&str> {
        // Sorted so `zftp session` listing is deterministic across
        // runs. Matches zsh's table-walk order for the underlying
        // sessions hash.
        let mut names: Vec<&str> = self.sessions.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }
}

/// `zftp` builtin entry point — C-faithful signature matching
/// `static int bin_zftp(char *name, char **args, Options ops, int func)`
/// from Src/Modules/zftp.c:3002. Acquires ZFTP_STATE, dispatches by
/// subcommand string, emits any captured output to stdout/stderr
/// based on status, returns the bare i32 status C's execbuiltin path
/// consumes.
#[allow(non_snake_case)]
pub fn bin_zftp(_nam: &str, args: &[String],                                 // c:3002
                _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    let argv: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut zftp_guard = ZFTP_STATE.lock()
        .unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let zftp = &mut *zftp_guard;
    let args = &argv[..];
    let (status, output): (i32, String) = (|| {
    if args.is_empty() {
        return (1, "zftp: subcommand required\n".to_string());
    }

    match args[0] {
        "open" => {
            if args.len() < 2 {
                return (1, "zftp open: host required\n".to_string());
            }

            let host = args[1];
            let port: Option<u16> = args.get(2).and_then(|s| s.parse().ok());

            let session_name = zftp.current_name().unwrap_or("default").to_string();

            let sess = zftp.create_session(&session_name);

            match sess.connect(host, port) {
                Ok(resp) => {
                    if (resp.0 >= 100 && resp.0 < 400) {
                        zftp.set_current(&session_name);
                        (0, resp.1)
                    } else {
                        (1, resp.1)
                    }
                }
                Err(e) => (1, format!("zftp open: {}\n", e)),
            }
        }

        "login" | "user" => {
            if args.len() < 2 {
                return (1, "zftp login: user required\n".to_string());
            }

            let user = args[1];
            let pass = args.get(2).copied();

            let sess = match zftp.get_session_mut(None) {
                Some(s) => s,
                None => return (1, "zftp login: not connected\n".to_string()),
            };

            match sess.login(user, pass) {
                Ok(resp) => {
                    if (resp.0 >= 200 && resp.0 < 300) {
                        (0, resp.1)
                    } else {
                        (1, resp.1)
                    }
                }
                Err(e) => (1, format!("zftp login: {}\n", e)),
            }
        }

        "cd" => {
            if args.len() < 2 {
                return (1, "zftp cd: path required\n".to_string());
            }

            let sess = match zftp.get_session_mut(None) {
                Some(s) => s,
                None => return (1, "zftp cd: not connected\n".to_string()),
            };

            match sess.cd(args[1]) {
                Ok(resp) => {
                    if (resp.0 >= 200 && resp.0 < 300) {
                        (0, resp.1)
                    } else {
                        (1, resp.1)
                    }
                }
                Err(e) => (1, format!("zftp cd: {}\n", e)),
            }
        }

        "cdup" => {
            let sess = match zftp.get_session_mut(None) {
                Some(s) => s,
                None => return (1, "zftp cdup: not connected\n".to_string()),
            };

            match sess.cdup() {
                Ok(resp) => {
                    if (resp.0 >= 200 && resp.0 < 300) {
                        (0, resp.1)
                    } else {
                        (1, resp.1)
                    }
                }
                Err(e) => (1, format!("zftp cdup: {}\n", e)),
            }
        }

        "pwd" => {
            let sess = match zftp.get_session_mut(None) {
                Some(s) => s,
                None => return (1, "zftp pwd: not connected\n".to_string()),
            };

            match sess.pwd() {
                Ok((resp, pwd)) => {
                    if let Some(p) = pwd {
                        (0, format!("{}\n", p))
                    } else {
                        (1, resp.1)
                    }
                }
                Err(e) => (1, format!("zftp pwd: {}\n", e)),
            }
        }

        "dir" | "ls" => {
            let path = args.get(1).copied();
            let use_nlst = args[0] == "ls";

            let sess = match zftp.get_session_mut(None) {
                Some(s) => s,
                None => return (1, "zftp dir: not connected\n".to_string()),
            };

            let result = if use_nlst {
                sess.nlst(path)
            } else {
                sess.list(path)
            };

            match result {
                Ok((resp, lines)) => {
                    if (resp.0 >= 200 && resp.0 < 300) {
                        (0, lines.join("\n") + "\n")
                    } else {
                        (1, resp.1)
                    }
                }
                Err(e) => (1, format!("zftp dir: {}\n", e)),
            }
        }

        "get" => {
            if args.len() < 2 {
                return (1, "zftp get: remote file required\n".to_string());
            }

            let remote = args[1];
            let local = args.get(2).unwrap_or(&remote);

            let sess = match zftp.get_session_mut(None) {
                Some(s) => s,
                None => return (1, "zftp get: not connected\n".to_string()),
            };

            match sess.get(remote, Path::new(local)) {
                Ok(resp) => {
                    if (resp.0 >= 200 && resp.0 < 300) {
                        (0, String::new())
                    } else {
                        (1, resp.1)
                    }
                }
                Err(e) => (1, format!("zftp get: {}\n", e)),
            }
        }

        "put" => {
            if args.len() < 2 {
                return (1, "zftp put: local file required\n".to_string());
            }

            let local = args[1];
            let remote = args.get(2).unwrap_or(&local);

            let sess = match zftp.get_session_mut(None) {
                Some(s) => s,
                None => return (1, "zftp put: not connected\n".to_string()),
            };

            match sess.put(Path::new(local), remote) {
                Ok(resp) => {
                    if (resp.0 >= 200 && resp.0 < 300) {
                        (0, String::new())
                    } else {
                        (1, resp.1)
                    }
                }
                Err(e) => (1, format!("zftp put: {}\n", e)),
            }
        }

        "delete" => {
            if args.len() < 2 {
                return (1, "zftp delete: file required\n".to_string());
            }

            let sess = match zftp.get_session_mut(None) {
                Some(s) => s,
                None => return (1, "zftp delete: not connected\n".to_string()),
            };

            match sess.delete(args[1]) {
                Ok(resp) => {
                    if (resp.0 >= 200 && resp.0 < 300) {
                        (0, String::new())
                    } else {
                        (1, resp.1)
                    }
                }
                Err(e) => (1, format!("zftp delete: {}\n", e)),
            }
        }

        "mkdir" => {
            if args.len() < 2 {
                return (1, "zftp mkdir: directory required\n".to_string());
            }

            let sess = match zftp.get_session_mut(None) {
                Some(s) => s,
                None => return (1, "zftp mkdir: not connected\n".to_string()),
            };

            match sess.mkdir(args[1]) {
                Ok(resp) => {
                    if (resp.0 >= 200 && resp.0 < 300) {
                        (0, String::new())
                    } else {
                        (1, resp.1)
                    }
                }
                Err(e) => (1, format!("zftp mkdir: {}\n", e)),
            }
        }

        "rmdir" => {
            if args.len() < 2 {
                return (1, "zftp rmdir: directory required\n".to_string());
            }

            let sess = match zftp.get_session_mut(None) {
                Some(s) => s,
                None => return (1, "zftp rmdir: not connected\n".to_string()),
            };

            match sess.rmdir(args[1]) {
                Ok(resp) => {
                    if (resp.0 >= 200 && resp.0 < 300) {
                        (0, String::new())
                    } else {
                        (1, resp.1)
                    }
                }
                Err(e) => (1, format!("zftp rmdir: {}\n", e)),
            }
        }

        "rename" => {
            if args.len() < 3 {
                return (1, "zftp rename: from and to required\n".to_string());
            }

            let sess = match zftp.get_session_mut(None) {
                Some(s) => s,
                None => return (1, "zftp rename: not connected\n".to_string()),
            };

            match sess.rename(args[1], args[2]) {
                Ok(resp) => {
                    if (resp.0 >= 200 && resp.0 < 300) {
                        (0, String::new())
                    } else {
                        (1, resp.1)
                    }
                }
                Err(e) => (1, format!("zftp rename: {}\n", e)),
            }
        }

        "type" | "ascii" | "binary" => {
            let transfer_type = match args[0] {
                "ascii" => ZFST_ASCI,
                "binary" => ZFST_IMAG,
                "type" => {
                    if args.len() < 2 {
                        let sess = match zftp.get_session(None) {
                            Some(s) => s,
                            None => return (1, "zftp type: not connected\n".to_string()),
                        };
                        return (
                            0,
                            format!(
                                "{}\n",
                                if sess.transfer_type == ZFST_ASCI {
                                    "ascii"
                                } else {
                                    "binary"
                                }
                            ),
                        );
                    }
                    match args[1].to_lowercase().as_str() {
                        "a" | "ascii" => ZFST_ASCI,
                        "i" | "binary" | "image" => ZFST_IMAG,
                        _ => return (1, format!("zftp type: unknown type {}\n", args[1])),
                    }
                }
                _ => unreachable!(),
            };

            let sess = match zftp.get_session_mut(None) {
                Some(s) => s,
                None => return (1, "zftp type: not connected\n".to_string()),
            };

            match sess.set_type(transfer_type) {
                Ok(resp) => {
                    if (resp.0 >= 200 && resp.0 < 300) {
                        (0, String::new())
                    } else {
                        (1, resp.1)
                    }
                }
                Err(e) => (1, format!("zftp type: {}\n", e)),
            }
        }

        "bslashquote" => {
            if args.len() < 2 {
                return (1, "zftp bslashquote: command required\n".to_string());
            }

            let cmd = args[1..].join(" ");

            let sess = match zftp.get_session_mut(None) {
                Some(s) => s,
                None => return (1, "zftp bslashquote: not connected\n".to_string()),
            };

            match sess.bslashquote(&cmd) {
                Ok(resp) => (if (resp.0 >= 100 && resp.0 < 400) { 0 } else { 1 }, resp.1),
                Err(e) => (1, format!("zftp bslashquote: {}\n", e)),
            }
        }

        "close" | "quit" => {
            let sess = match zftp.get_session_mut(None) {
                Some(s) => s,
                None => return (0, String::new()),
            };

            match sess.close() {
                Ok(_) => (0, String::new()),
                Err(e) => (1, format!("zftp close: {}\n", e)),
            }
        }

        "session" => {
            if args.len() < 2 {
                let names = zftp.session_names();
                let current = zftp.current_name();
                let mut out = String::new();
                for name in names {
                    let marker = if Some(name) == current { "* " } else { "  " };
                    out.push_str(&format!("{}{}\n", marker, name));
                }
                return (0, out);
            }

            let name = args[1];
            if zftp.sessions.contains_key(name) {
                zftp.set_current(name);
            } else {
                zftp.create_session(name);
                zftp.set_current(name);
            }
            (0, String::new())
        }

        "rmsession" => {
            if args.len() < 2 {
                return (1, "zftp rmsession: session name required\n".to_string());
            }

            if zftp.remove_session(args[1]).is_some() {
                (0, String::new())
            } else {
                (
                    1,
                    format!("zftp rmsession: session {} not found\n", args[1]),
                )
            }
        }

        "test" => {
            let sess = zftp.get_session(None);
            if sess.map(|s| s.connected).unwrap_or(false) {
                (0, String::new())
            } else {
                (1, String::new())
            }
        }

        _ => (1, format!("zftp: unknown subcommand {}\n", args[0])),
    }
    })();
    drop(zftp_guard);
    if !output.is_empty() {
        if status == 0 { print!("{}", output); } else { eprint!("{}", output); }
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transfer_type() {
        // Inline-test pattern matching C: `(typ & ZFST_IMAG) ? "I" : "A"`
        let ascii_letter = if (ZFST_ASCI & ZFST_IMAG) != 0 { "I" } else { "A" };
        let image_letter = if (ZFST_IMAG & ZFST_IMAG) != 0 { "I" } else { "A" };
        assert_eq!(ascii_letter, "A");
        assert_eq!(image_letter, "I");
    }

    #[test]
    fn test_transfer_mode() {
        // Inline-test pattern matching C: `(mode & ZFST_BLOC) ? "B" : "S"`
        let stream_letter = if (ZFST_STRE & ZFST_BLOC) != 0 { "B" } else { "S" };
        let block_letter  = if (ZFST_BLOC & ZFST_BLOC) != 0 { "B" } else { "S" };
        assert_eq!(stream_letter, "S");
        assert_eq!(block_letter, "B");
    }

    /// FTP reply-code class predicates per RFC 959. C tests these
    /// inline at every reply-check call site (e.g. `if (lastcode < 400)`).
    fn is_positive(c: i32) -> bool { c >= 100 && c < 400 }
    fn is_positive_completion(c: i32) -> bool { c >= 200 && c < 300 }
    fn is_positive_intermediate(c: i32) -> bool { c >= 300 && c < 400 }
    fn is_negative(c: i32) -> bool { c >= 400 }

    #[test]
    fn test_ftp_response_positive() {
        let resp: FtpResponse = (200, "OK".to_string());
        assert!(is_positive(resp.0));
        assert!(is_positive_completion(resp.0));
        assert!(!is_negative(resp.0));
    }

    #[test]
    fn test_ftp_response_intermediate() {
        let resp: FtpResponse = (331, "Password required".to_string());
        assert!(is_positive(resp.0));
        assert!(is_positive_intermediate(resp.0));
        assert!(!is_positive_completion(resp.0));
    }

    #[test]
    fn test_ftp_response_negative() {
        let resp: FtpResponse = (550, "File not found".to_string());
        assert!(is_negative(resp.0));
        assert!(!is_positive(resp.0));
    }

    #[test]
    fn test_ftp_session_new() {
        let sess = zftp_session::new("test");
        assert_eq!(sess.name, "test");
        assert!(!sess.connected);
        assert!(!sess.logged_in);
    }

    #[test]
    fn test_parse_pasv_response() {
        let msg = "227 Entering Passive Mode (192,168,1,1,4,1)";
        let (ip, port) = zfopendata(msg).unwrap();
        assert_eq!(ip, "192.168.1.1");
        assert_eq!(port, 1025);
    }

    #[test]
    fn test_parse_pasv_response_invalid() {
        let msg = "invalid";
        assert!(zfopendata(msg).is_err());
    }

    #[test]
    fn test_zftp_new() {
        let zftp = Zftp::new();
        assert!(zftp.session_names().is_empty());
    }

    #[test]
    fn test_zftp_create_session() {
        let mut zftp = Zftp::new();
        zftp.create_session("test");
        assert!(zftp.sessions.contains_key("test"));
    }

    #[test]
    fn test_zftp_remove_session() {
        let mut zftp = Zftp::new();
        zftp.create_session("test");
        assert!(zftp.remove_session("test").is_some());
        assert!(zftp.remove_session("test").is_none());
    }

    #[test]
    fn test_zftp_set_current() {
        let mut zftp = Zftp::new();
        zftp.create_session("test");
        assert!(zftp.set_current("test"));
        assert!(!zftp.set_current("nonexistent"));
    }

    #[test]
    fn test_builtin_zftp_no_args() {
        let mut zftp = Zftp::new();
        let status = bin_zftp("zftp", &[].iter().map(|s: &&str| s.to_string()).collect::<Vec<_>>(), &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
        assert_eq!(status, 1);
    }

    /// Port of `zftp_open()` from `Src/Modules/zftp.c:1690`.
    #[test]
    fn test_builtin_zftp_session() {
        // Reset global state for test isolation.
        zftp_cleanup();
        let status = bin_zftp("zftp", &["session", "test"].iter().map(|s: &&str| s.to_string()).collect::<Vec<_>>(), &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
        assert_eq!(status, 0);
        assert!(ZFTP_STATE.lock().unwrap().sessions.contains_key("test"));
        zftp_cleanup();
    }

    #[test]
    fn test_builtin_zftp_test_not_connected() {
        let mut zftp = Zftp::new();
        let status = bin_zftp("zftp", &["test"].iter().map(|s: &&str| s.to_string()).collect::<Vec<_>>(), &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
        assert_eq!(status, 1);
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// =====================================================================
// static struct features module_features                            c:3163 (zftp.c)
// =====================================================================

use std::sync::{Mutex, OnceLock};
use crate::ported::zsh_h::{features as features_t, module};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None, bn_size: 1,                                       // bintab[1]: zftp
        cd_list: None, cd_size: 0,
        mf_list: None, mf_size: 0,
        pd_list: None, pd_size: 0,
        n_abstract: 0,
    }))
}

/// Port of `setup_()` from `Src/Modules/zftp.c:3174`.
pub fn setup_(_m: *const module) -> i32 {                                    // c:3174
    // C body c:3176-3177 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_()` from `Src/Modules/zftp.c:3181`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_()` from `Src/Modules/zftp.c:3189`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_()` from `Src/Modules/zftp.c:3196`.
pub fn boot_(_m: *const module) -> i32 {                                     // c:3196
    // C body c:3198-3214:
    //   off_t tmout_def = 60;
    //   zfsetparam("ZFTP_VERBOSE", "450", ZFPM_IFUNSET);
    //   zfsetparam("ZFTP_TMOUT", &tmout_def, ZFPM_IFUNSET|ZFPM_INTEGER);
    //   zfsetparam("ZFTP_PREFS", "PS", ZFPM_IFUNSET);
    //   zfprefs = ZFPF_SNDP|ZFPF_PASV;
    //   zfsessions = znewlinklist(); newsession("default");
    //   addhookfunc("exit", zftpexithook);
    zfsetparam("ZFTP_VERBOSE", "450", 0);                                    // c:3203
    zfsetparam("ZFTP_TMOUT", "60", 0);                                       // c:3204
    zfsetparam("ZFTP_PREFS", "PS", 0);                                       // c:3205
    let _default = newsession("default");                                    // c:3210
    0
}

/// Port of `cleanup_()` from `Src/Modules/zftp.c:3219`.
pub fn cleanup_(m: *const module) -> i32 {
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_()` from `Src/Modules/zftp.c:3228`.
pub fn finish_(_m: *const module) -> i32 {                                   // c:3228
    // C body c:3230-3231 — `return 0`. Faithful empty-body port; the
    //                      cleanup of zfsessions happens in cleanup_.
    0
}

fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["b:zftp".to_string()]
}
fn handlefeatures(m: *const module, f: &Mutex<features_t>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() { *enables = Some(getfeatureenables(m, f)); }
    else if let Some(e) = enables.as_ref() { return setfeatureenables(m, f, Some(e)); }
    0
}
fn getfeatureenables(_m: *const module, f: &Mutex<features_t>) -> Vec<i32> {
    let g = f.lock().unwrap();
    vec![0; (g.bn_size + g.cd_size + g.mf_size + g.pd_size + g.n_abstract) as usize]
}
// File-static delegator to `Src/module.c:3349 setfeatureenables` —
// dispatches per-feature enable bits through setbuiltins/setconddefs/
// setmathfuncs/setparamdefs. The static-link Rust path treats every
// feature as always-enabled, so this no-op return matches what
// cleanup_(NULL) needs (revoke nothing).
fn setfeatureenables(_m: *const module, _f: &Mutex<features_t>, _e: Option<&Vec<i32>>) -> i32 { 0 }

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/zftp.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `freesession()` from `Src/Modules/zftp.c:2874`.
/// C: `static void freesession(Zftp_session sptr)` — release `sptr`'s
/// name + params + userparams + the struct itself.
#[allow(non_snake_case)]
pub fn freesession(sptr: &mut zftp_session) {                                 // c:2874
    // c:2877 — zsfree(sptr->name);
    sptr.name.clear();
    // c:2878-2881 — walk zfparams + sptr->params freeing each param value.
    sptr.params.clear();
    // c:2882-2883 — if (sptr->userparams) freearray(sptr->userparams);
    sptr.userparams.clear();
    // c:2884 — zfree(sptr, sizeof(struct zftp_session)); the caller's
    // owning Box::drop releases the struct memory.
}

/// Port of `newsession()` from `Src/Modules/zftp.c:2803`.
/// C: `static Zftp_session newsession(char *nm)`.
#[allow(non_snake_case)]
pub fn newsession(nm: &str) -> Box<zftp_session> {
    Box::new(zftp_session::new(nm))
}

/// Port of `savesession()` from `Src/Modules/zftp.c:2832`.
/// C: `static void savesession(void)` — copy each ZFTP_* shell param
/// into zfsess->params so session-switching preserves the values.
#[allow(non_snake_case)]
pub fn savesession() {                                                        // c:2832
    // c:2834 — char **ps, **pd, *val; (Rust uses indexing over slices)
    let val: String;
    let _ = val;

    if let Ok(mut state) = ZFTP_STATE.lock() {
        let sess = match state.get_session_mut(None) {
            Some(s) => s,
            None => return,
        };
        // c:2836-2845 — for each zfparams[i], copy the current shell param.
        sess.params.clear();
        for ps in ZFPARAMS {                                                  // c:2836
            // c:2840 — val = getsparam(*ps);
            // Static-link path: read from process env, the closest analog
            // until paramtab is bucket-2-consolidated. Matches the
            // `getsparam` body in src/ported/modules/ksh93.rs:537.
            let val = std::env::var(ps).unwrap_or_default();
            // c:2841 / c:2843 — *pd = ztrdup(val) or NULL.
            sess.params.push(val);
        }
        // c:2846 — *pd = NULL; (terminator) — Rust Vec is self-terminating.
    }
}

/// Port of `switchsession()` from `Src/Modules/zftp.c:2856`.
/// C: `static void switchsession(char *nm)`.
#[allow(non_snake_case)]
pub fn switchsession(nm: &str) {
    if let Ok(mut state) = ZFTP_STATE.lock() {
        // C: walks zfsessions list for matching `nm`; if missing,
        // creates one. Static-link path: register-or-create on the Zftp wrapper.
        let _ = state.create_session(nm);
        state.set_current(nm);
    }
}

/// Port of `zfalarm()` from `Src/Modules/zftp.c:384`.
/// C: `static void zfalarm(int tmout)` — set up alarm + SIGALRM handler.
#[allow(non_snake_case)]
pub fn zfalarm(tmout: i32) {                                                  // c:384
    ZFDRRRRING.store(0, std::sync::atomic::Ordering::Relaxed);                // c:386
    // c:387-392 — fire alarm even when tmout is 0 so a pending non-zero
    // main-shell alarm doesn't bleed into the FTP code path.
    if ZFALARMED.load(std::sync::atomic::Ordering::Relaxed) != 0 {            // c:393
        unsafe { libc::alarm(tmout as u32); }                                 // c:394
        return;                                                               // c:395
    }
    // c:397 — signal(SIGALRM, zfhandler);
    unsafe {
        libc::signal(libc::SIGALRM, zfhandler as libc::sighandler_t);
    }
    // c:398 — oalremain = alarm(tmout);
    let oalremain = unsafe { libc::alarm(tmout as u32) };
    OALREMAIN.store(oalremain, std::sync::atomic::Ordering::Relaxed);
    if oalremain != 0 {                                                       // c:399
        // c:400 — oaltime = zmonotime(NULL);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        OALTIME.store(now, std::sync::atomic::Ordering::Relaxed);
    }
    ZFALARMED.store(1, std::sync::atomic::Ordering::Relaxed);                 // c:405
}

/// Port of `zfargstring()` from `Src/Modules/zftp.c:546`.
/// C: `char *zfargstring(char *cmd, char **args)` — joins cmd + args.
#[allow(non_snake_case)]
pub fn zfargstring(cmd: &str, args: &[&str]) -> String {
    // c:546-570 — zhalloc + sprintf joining cmd + space-sep args.
    let mut s = cmd.to_string();
    for a in args {
        s.push(' ');
        s.push_str(a);
    }
    s
}

/// Port of `zfclose()` from `Src/Modules/zftp.c:2711`.
/// C: `void zfclose(int leaveparams)` — closes connection.
#[allow(non_snake_case)]
pub fn zfclose(_leaveparams: i32) {
    if let Ok(mut state) = ZFTP_STATE.lock() {
        if let Some(sess) = state.get_session_mut(None) {
            sess.cin = None;
            sess.control = None;
            sess.dfd = -1;
            sess.connected = false;
            sess.logged_in = false;
        }
        ZFNOPEN.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Port of `zfclosedata()` from `Src/Modules/zftp.c:1043`.
/// C: `static void zfclosedata(void)` — closes data fd only.
#[allow(non_snake_case)]
pub fn zfclosedata() {
    if let Ok(mut state) = ZFTP_STATE.lock() {
        if let Some(sess) = state.get_session_mut(None) {
            sess.dfd = -1;                                                // c:1043-1051 close(dfd)
        }
    }
}

/// Port of `zfendtrans()` from `Src/Modules/zftp.c:1295`.
/// C: `static void zfendtrans(void)` — unsets the ZFTP_* transfer params.
#[allow(non_snake_case)]
pub fn zfendtrans() {                                                         // c:1295
    zfunsetparam("ZFTP_SIZE");                                                // c:1297
    zfunsetparam("ZFTP_FILE");                                                // c:1298
    zfunsetparam("ZFTP_TRANSFER");                                            // c:1299
    zfunsetparam("ZFTP_COUNT");                                               // c:1300
}

/// Port of `zfgetcwd()` from `Src/Modules/zftp.c:2358`.
/// C: `static int zfgetcwd(void)` — sends PWD, parses reply.
#[allow(non_snake_case)]
pub fn zfgetcwd() -> i32 {
    let _ = zfsendcmd("PWD\r\n");
    if zfgetmsg() == 0 && lastcode.load(std::sync::atomic::Ordering::Relaxed) >= 200 { 0 } else { 1 }
}

/// Port of `zfgetdata()` from `Src/Modules/zftp.c:1065`.
/// C: `static int zfgetdata(char *name, char *rest, char *cmd, int getsize)` —
/// open the data connection (PASV path), optionally send REST, then
/// send the transfer command. Returns 0 on success, 1 on failure.
#[allow(non_snake_case)]
pub fn zfgetdata(name: &str, rest: &str, cmd: &str, getsize: i32) -> i32 {    // c:1065
    // c:1067-1069 — locals at fn top.
    // C: ZSOCKLEN_T len; int newfd, is_passive; union tcp_sockaddr zdsock;
    // PASV-only path: len + newfd are unused (no accept(2) for PORT mode).
    let is_passive: bool;                                                     // c:1068

    // c:1071-1072 — zfopendata(name, &zdsock, &is_passive). The C zfopendata
    // is a 200-line bind+listen+PORT setup; Rust port-equivalent uses the
    // existing PASV-only helper. Send PASV, parse the (h1,h2,h3,h4,p1,p2)
    // response into (ip, port), then connect TcpStream.
    if zfsendcmd("PASV\r\n") > 2 {                                            // c:881 zfsendcmd(psv_cmd)
        return 1;                                                             // c:882
    }
    is_passive = true;

    // Parse the (h1,h2,h3,h4,p1,p2) tuple from lastmsg.
    let last = lastmsg.lock().ok().map(|m| m.clone()).unwrap_or_default();
    let (ip, port) = match zfopendata(&last) {
        Ok(t) => t,
        Err(_) => {
            crate::ported::utils::zwarnnam(name, "bad PASV response");
            return 1;
        }
    };

    // Connect to the data port. Replaces C's socket()+connect() pair
    // at c:865-869 + the post-PASV connect.
    let addr = format!("{}:{}", ip, port);
    let data_stream = match std::net::TcpStream::connect(&addr) {
        Ok(s) => s,
        Err(_) => {
            crate::ported::utils::zwarnnam(name, "can't open data socket");
            return 1;
        }
    };
    use std::os::unix::io::AsRawFd;
    let dfd_raw = data_stream.as_raw_fd();

    // c:1084-1087 — REST command for resume.
    if !rest.is_empty() && zfsendcmd(rest) > 3 {
        zfclosedata();
        return 1;
    }

    // c:1089-1092 — send the transfer command (RETR / STOR / etc.).
    if zfsendcmd(cmd) > 2 {                                                   // c:1089
        zfclosedata();                                                        // c:1090
        return 1;                                                             // c:1091
    }

    // c:1093-1116 — parse "Opening data connection for file (N bytes)"
    // hint to populate ZFTP_SIZE without a separate SIZE request.
    if getsize != 0 || cmd.starts_with("RETR") {
        let cur_last = lastmsg.lock().ok().map(|m| m.clone()).unwrap_or_default();
        if let Some(byte_idx) = cur_last.find("bytes") {                      // c:1101
            // Walk backward to find the start of the digit run.
            let prefix = &cur_last[..byte_idx];
            let trimmed: String = prefix
                .chars()
                .rev()
                .skip_while(|c| !c.is_ascii_digit())
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            if !trimmed.is_empty() && getsize != 0 {
                zfsetparam("ZFTP_SIZE", &trimmed, ZFPM_READONLY | ZFPM_INTEGER); // c:1112
            }
        }
    }

    // c:1118-1143 — PORT-mode accept handling. Rust port is PASV-only;
    // when passive the dfd we have is already the data fd.
    let _ = is_passive;
    // Store the connected dfd. zfmovefd would dup past stdio fds; for now
    // just keep the raw fd we got from std::net.
    if let Ok(mut state) = ZFTP_STATE.lock() {
        if let Some(sess) = state.get_session_mut(None) {
            sess.dfd = zfmovefd(dfd_raw);                                     // c:1142
        }
    }
    // Keep TcpStream alive past this fn so the fd doesn't close.
    // The fd is owned by the session now; transfer code reads via dfd.
    std::mem::forget(data_stream);

    // c:1156-1163 — SO_LINGER 120s.
    let li = libc::linger { l_onoff: 1, l_linger: 120 };
    unsafe {
        libc::setsockopt(dfd_raw, libc::SOL_SOCKET, libc::SO_LINGER,
                         &li as *const _ as *const libc::c_void,
                         std::mem::size_of::<libc::linger>() as libc::socklen_t);
    }
    // c:1167-1170 — IP_TOS = IPTOS_THROUGHPUT.
    let tos: libc::c_int = 0x08;                                              // IPTOS_THROUGHPUT
    unsafe {
        libc::setsockopt(dfd_raw, libc::IPPROTO_IP, libc::IP_TOS,
                         &tos as *const _ as *const libc::c_void,
                         std::mem::size_of::<libc::c_int>() as libc::socklen_t);
    }
    // c:1174 — fcntl(dfd, F_SETFD, FD_CLOEXEC).
    unsafe { libc::fcntl(dfd_raw, libc::F_SETFD, libc::FD_CLOEXEC); }

    0                                                                         // c:1177
}

/// Port of `zfgetinfo()` from `Src/Modules/zftp.c:1999`.
/// C: `static char * zfgetinfo(char *prompt, int noecho)` — prompt
/// the tty (echoing or with ECHO masked off for passwords) and read
/// one line of input.
#[allow(non_snake_case)]
pub fn zfgetinfo(prompt: &str, noecho: i32) -> Option<String> {              // c:1999
    use std::io::{BufRead, Write};
    // c:2001-2006 — locals.
    let mut resettty: i32 = 0;                                                // c:2001
    let mut instr = String::new();                                            // c:2005 char instr[256]
    let len: usize = 0;                                                       // c:2006 (unused in Rust path)
    let _ = len;

    let saved_termios: Option<libc::termios>;

    // c:2013 — if (isatty(0)) prompt + tty setup.
    if unsafe { libc::isatty(0) } != 0 {                                      // c:2013
        if noecho != 0 {                                                      // c:2014
            // c:2024-2032 — copy current termios, clear ECHO, install.
            let mut ti: libc::termios = unsafe { std::mem::zeroed() };
            if unsafe { libc::tcgetattr(0, &mut ti) } == 0 {
                saved_termios = Some(ti);
                ti.c_lflag &= !libc::ECHO;                                    // c:2028
                unsafe { libc::tcsetattr(0, libc::TCSANOW, &ti); }            // c:2032
                resettty = 1;                                                 // c:2033
            } else {
                saved_termios = None;
            }
        } else {
            saved_termios = None;
        }
        // c:2035-2037 — fflush(stdin) + write prompt to stderr.
        eprint!("{}", prompt);                                                // c:2036
        let _ = std::io::stderr().flush();                                    // c:2037
    } else {
        saved_termios = None;
    }

    // c:2040-2043 — fgets(instr, 256, stdin); strip trailing \n.
    let stdin = std::io::stdin();
    let mut handle = stdin.lock();
    match handle.read_line(&mut instr) {                                      // c:2040
        Ok(0) => instr.clear(),                                               // c:2041 NULL → empty
        Ok(_) => {                                                            // c:2042-2043 strip \n
            if instr.ends_with('\n') {
                instr.pop();
            }
        }
        Err(_) => instr.clear(),
    }

    // c:2045 — strret = dupstring(instr); (just keep instr as the result)
    let strret = instr.clone();

    // c:2047-2052 — restore termios if we modified it.
    if resettty != 0 {                                                        // c:2047
        println!();                                                           // c:2049 '\n' didn't echo
        let _ = std::io::stdout().flush();                                    // c:2050
        if let Some(ti) = saved_termios {                                     // c:2051
            unsafe { libc::tcsetattr(0, libc::TCSANOW, &ti); }
        }
    }

    Some(strret)                                                              // c:2054
}

/// Port of `zfgetline()` from `Src/Modules/zftp.c:571`.
/// C: `int zfgetline(char *ln, int lnsize, int tmout)` — read a single
/// CRLF-terminated line from the control connection, handling TELNET
/// IAC command escapes and SIGALRM-driven timeout.
#[allow(non_snake_case)]
pub fn zfgetline(ln: &mut [u8], lnsize: i32, tmout: i32) -> i32 {             // c:571
    use std::io::Read;
    // c:573-575 — locals at function top (Rule 5).
    let mut ch: i32;                                                          // c:573 int ch
    let mut added: i32 = 0;                                                   // c:573 added
    // c:575 — char *pcur = ln, cmdbuf[3];
    let mut pcur: usize = 0;                                                  // pointer index into ln
    let mut cmdbuf: [u8; 3] = [0; 3];

    ZCFINISH.store(0, std::sync::atomic::Ordering::Relaxed);                  // c:577 zcfinish = 0
    let lnsize = lnsize - 1;                                                  // c:579 leave room for null
    if !ln.is_empty() {
        ln[0] = 0;                                                            // c:581 ln[0] = '\0'
    }

    // c:583-587 — setjmp guard via ZFDRRRRING flag.
    if ZFDRRRRING.load(std::sync::atomic::Ordering::Relaxed) != 0 {           // c:583
        unsafe { libc::alarm(0); }                                            // c:584
        crate::ported::utils::zwarnnam("zftp", "timeout getting response");   // c:585
        return 6;                                                             // c:586
    }
    zfalarm(tmout);                                                           // c:588

    // c:597-678 — for (;;) read loop with TELNET IAC handling.
    let mut state = match ZFTP_STATE.lock() {
        Ok(s) => s,
        Err(_) => return 6,
    };
    let sess = match state.get_session_mut(None) {
        Some(s) => s,
        None => return 6,
    };
    let stream = match sess.cin.as_mut() {
        Some(s) => s,
        None => return 6,
    };
    let mut byte = [0u8; 1];

    'main: loop {                                                             // c:597 for (;;)
        // c:598 — ch = fgetc(zfsess->cin);
        ch = match stream.read(&mut byte) {
            Ok(0) => -1,                                                      // EOF
            Ok(_) => byte[0] as i32,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,// c:602 EINTR retry
            Err(_) => -1,
        };

        match ch {
            -1 => {                                                           // c:601 EOF
                ZCFINISH.store(2, std::sync::atomic::Ordering::Relaxed);      // c:606
            }
            0x0d => {                                                         // c:609 '\r'
                ch = match stream.read(&mut byte) {                           // c:611
                    Ok(0) => -1,
                    Ok(_) => byte[0] as i32,
                    Err(_) => -1,
                };
                if ch == -1 {                                                 // c:612 EOF
                    ZCFINISH.store(2, std::sync::atomic::Ordering::Relaxed);  // c:613
                } else if ch == 0x0a {                                        // c:616 '\n'
                    ZCFINISH.store(1, std::sync::atomic::Ordering::Relaxed);  // c:617
                } else if ch == 0x00 {                                        // c:620 '\0'
                    ch = 0x0d;                                                // c:621
                } else {
                    ch = 0x0d;                                                // c:625
                }
            }
            0x0a => {                                                         // c:628 '\n' (unexpected)
                ZCFINISH.store(1, std::sync::atomic::Ordering::Relaxed);      // c:630
            }
            255 => {                                                          // c:633 IAC
                ch = match stream.read(&mut byte) {                           // c:638
                    Ok(0) => -1,
                    Ok(_) => byte[0] as i32,
                    Err(_) => -1,
                };
                match ch {
                    251 | 252 => {                                            // c:640-641 WILL/WONT
                        ch = match stream.read(&mut byte) {                   // c:642
                            Ok(0) => -1,
                            Ok(_) => byte[0] as i32,
                            Err(_) => -1,
                        };
                        cmdbuf[0] = 255;                                      // c:644 IAC
                        cmdbuf[1] = 254;                                      // c:645 DONT
                        cmdbuf[2] = ch as u8;                                 // c:646
                        // c:647 — write_loop(zfsess->control->fd, cmdbuf, 3);
                        if let Some(ctrl) = sess.control.as_mut() {
                            use std::io::Write;
                            let _ = ctrl.write_all(&cmdbuf);
                        }
                        continue 'main;                                       // c:648
                    }
                    253 | 254 => {                                            // c:650-651 DO/DONT
                        ch = match stream.read(&mut byte) {                   // c:652
                            Ok(0) => -1,
                            Ok(_) => byte[0] as i32,
                            Err(_) => -1,
                        };
                        cmdbuf[0] = 255;                                      // c:654 IAC
                        cmdbuf[1] = 252;                                      // c:655 WONT
                        cmdbuf[2] = ch as u8;                                 // c:656
                        if let Some(ctrl) = sess.control.as_mut() {
                            use std::io::Write;
                            let _ = ctrl.write_all(&cmdbuf);
                        }
                        continue 'main;                                       // c:658
                    }
                    -1 => {                                                   // c:660 EOF
                        ZCFINISH.store(2, std::sync::atomic::Ordering::Relaxed); // c:662
                    }
                    _ => {}                                                   // c:665 default
                }
            }
            _ => {}
        }

        // c:671-672 — if (zcfinish) break;
        if ZCFINISH.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            break;
        }
        // c:673-676 — if (added < lnsize) { *pcur++ = ch; added++; }
        if added < lnsize && pcur < ln.len() {
            ln[pcur] = ch as u8;
            pcur += 1;
            added += 1;
        }
        // c:677 — junk if no room, keep reading.
    }

    unsafe { libc::alarm(0); }                                                // c:680
    if pcur < ln.len() {
        ln[pcur] = 0;                                                         // c:682 *pcur = '\0'
    }
    // c:684 — return (zcfinish & 2);
    ZCFINISH.load(std::sync::atomic::Ordering::Relaxed) & 2
}

/// Port of `zfgetmsg()` from `Src/Modules/zftp.c:702`.
/// C: `static int zfgetmsg(void)` — read a complete FTP server reply
/// (possibly multi-line), parse the 3-digit code, update lastcode +
/// lastcodestr + lastmsg + ZFTP_REPLY, return the first-digit status
/// (1/2/3/4/5) or 6 on error/disconnect.
#[allow(non_snake_case)]
pub fn zfgetmsg() -> i32 {                                                    // c:702
    // c:704-705 — char line[256], *ptr, *verbose;
    //             int stopit, printing = 0, tmout;
    let mut line = [0u8; 256];
    let mut printing: i32 = 0;
    let stopit_initial: bool;
    let tmout: i32;

    // c:707-708 — if (!zfsess->control) return 6;
    {
        let state = match ZFTP_STATE.lock() {
            Ok(s) => s,
            Err(_) => return 6,
        };
        let sess = match state.get_session(None) {
            Some(s) => s,
            None => return 6,
        };
        if sess.control.is_none() {
            return 6;                                                         // c:708
        }
    }

    // c:709-710 — zsfree(lastmsg); lastmsg = NULL;
    if let Ok(mut m) = lastmsg.lock() {
        m.clear();
    }

    // c:712 — tmout = getiparam("ZFTP_TMOUT");
    tmout = std::env::var("ZFTP_TMOUT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // c:714 — zfgetline(line, 256, tmout);
    zfgetline(&mut line, 256, tmout);
    // c:715 — ptr = line; (use string slice + offset index instead)
    let mut ptr_off: usize = 0;
    let line_str = std::str::from_utf8(&line)
        .unwrap_or("")
        .trim_end_matches('\0')
        .to_string();

    // c:716 — if (zfdrrrring || !idigit(ptr[0..3])) — timeout or not FTP.
    let is_digit = |b: u8| b.is_ascii_digit();
    let timeout_or_bad = ZFDRRRRING.load(std::sync::atomic::Ordering::Relaxed) != 0
        || line.len() < 3
        || !is_digit(line[0])
        || !is_digit(line[1])
        || !is_digit(line[2]);
    if timeout_or_bad {                                                       // c:716
        ZCFINISH.store(2, std::sync::atomic::Ordering::Relaxed);              // c:718
        if ZFCLOSING.load(std::sync::atomic::Ordering::Relaxed) == 0 {        // c:719
            zfclose(0);                                                       // c:720
        }
        if let Ok(mut m) = lastmsg.lock() { m.clear(); }                      // c:721
        if let Ok(mut cs) = lastcodestr.lock() {                              // c:722
            cs.copy_from_slice(b"000\0");
        }
        zfsetparam("ZFTP_REPLY", "", ZFPM_READONLY);                          // c:723
        return 6;                                                             // c:724
    }

    // c:726-729 — extract first 3 bytes into lastcodestr, parse to int.
    let code_str: String = std::str::from_utf8(&line[..3]).unwrap_or("0").to_string();
    if let Ok(mut cs) = lastcodestr.lock() {
        cs[0] = line[0]; cs[1] = line[1]; cs[2] = line[2]; cs[3] = 0;
    }
    let code: i32 = code_str.parse().unwrap_or(0);
    lastcode.store(code, std::sync::atomic::Ordering::Relaxed);
    ptr_off += 3;
    // c:730 — zfsetparam("ZFTP_CODE", lastcodestr, ZFPM_READONLY);
    zfsetparam("ZFTP_CODE", &code_str, ZFPM_READONLY);
    // c:731 — stopit = (*ptr++ != '-');
    stopit_initial = line.get(ptr_off).copied() != Some(b'-');
    ptr_off += 1;
    let mut stopit = stopit_initial;

    // c:733-744 — verbose check + initial-line printing.
    let verbose = std::env::var("ZFTP_VERBOSE").unwrap_or_default();          // c:734
    if verbose.contains(line[0] as char) {                                    // c:736
        printing = 1;                                                         // c:738
        eprint!("{}", line_str);                                              // c:739
    } else if verbose.contains('0') && !stopit {                              // c:740
        printing = 2;                                                         // c:742
        eprint!("{}", &line_str[ptr_off..]);                                  // c:743
    }
    if printing != 0 {                                                        // c:746
        eprintln!();                                                          // c:747
    }

    // c:749-775 — multi-line continuation loop.
    while ZCFINISH.load(std::sync::atomic::Ordering::Relaxed) != 2 && !stopit {
        line.fill(0);                                                         // reset
        ptr_off = 0;
        zfgetline(&mut line, 256, tmout);                                     // c:750
        if ZFDRRRRING.load(std::sync::atomic::Ordering::Relaxed) != 0 {       // c:752
            line[0] = 0;                                                      // c:753
            break;                                                            // c:754
        }
        // c:757-764 — code-prefix check.
        if &line[..3] == &code_str.as_bytes()[..3] {                          // c:757
            if line[3] == b' ' {                                              // c:758
                stopit = true;                                                // c:759
                ptr_off = 4;                                                  // c:760
            } else if line[3] == b'-' {                                       // c:761
                ptr_off = 4;                                                  // c:762
            }
        } else if &line[..4] == b"    " {                                     // c:763
            ptr_off = 4;                                                      // c:764
        }

        // c:766-774 — print intermediate line per `printing` mode.
        let cont_line = std::str::from_utf8(&line)
            .unwrap_or("")
            .trim_end_matches('\0');
        if printing == 2 {                                                    // c:766
            if !stopit {                                                      // c:767
                eprintln!("{}", &cont_line[ptr_off..]);                       // c:768-769
            }
        } else if printing != 0 {                                             // c:771
            eprintln!("{}", cont_line);                                       // c:772-773
        }
    }

    // c:777-778 — fflush(stderr);
    if printing != 0 {
        use std::io::Write;
        let _ = std::io::stderr().flush();
    }

    // c:781 — lastmsg = ztrdup(ptr);  (the trailing portion of last line)
    let last_msg_str: String = std::str::from_utf8(&line)
        .unwrap_or("")
        .trim_end_matches('\0')
        .chars()
        .skip(ptr_off)
        .collect();
    if let Ok(mut m) = lastmsg.lock() {
        *m = last_msg_str.clone();
    }
    // c:785 — zfsetparam("ZFTP_REPLY", ztrdup(line), ZFPM_READONLY);
    let whole_line = std::str::from_utf8(&line)
        .unwrap_or("")
        .trim_end_matches('\0');
    zfsetparam("ZFTP_REPLY", whole_line, ZFPM_READONLY);

    // c:791-797 — EOF or 421: close + warn.
    let zcfin = ZCFINISH.load(std::sync::atomic::Ordering::Relaxed);
    let cur_code = lastcode.load(std::sync::atomic::Ordering::Relaxed);
    if (zcfin == 2 || cur_code == 421)
        && ZFCLOSING.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        ZCFINISH.store(2, std::sync::atomic::Ordering::Relaxed);              // c:792
        zfclose(0);                                                           // c:793
        crate::ported::utils::zwarnnam("zftp",                                // c:795
            "remote server has closed connection");
        return 6;                                                             // c:796
    }
    // c:798-801 — 530 not-logged-in.
    if cur_code == 530 {                                                      // c:798
        return 6;                                                             // c:800
    }
    // c:807-810 — 120 wait-and-retry.
    if cur_code == 120 {                                                      // c:807
        crate::ported::utils::zwarnnam("zftp",                                // c:808
            &format!("delay expected, waiting: {}", last_msg_str));
        return zfgetmsg();                                                    // c:809
    }
    // c:813 — return lastcodestr[0] - '0';
    (code_str.as_bytes()[0] - b'0') as i32
}

/// Port of `zfhandler()` from `Src/Modules/zftp.c:366`.
/// C: `static void zfhandler(int sig)` — SIGALRM handler. Sets the
/// `zfdrrrring` flag so the next zfread/zfgetline returns -1 and exits
/// its setjmp-protected critical section.
#[allow(non_snake_case)]
pub extern "C" fn zfhandler(sig: i32) {                                       // c:366
    if sig == libc::SIGALRM {                                                 // c:368
        ZFDRRRRING.store(1, std::sync::atomic::Ordering::Relaxed);            // c:369
        // c:370-374 — errno = ETIMEDOUT (or EIO).
        unsafe {
            *libc::__error() = libc::ETIMEDOUT;
        }
        // c:375 — longjmp(zfalrmbuf, 1). Rust port doesn't use setjmp;
        // the ZFDRRRRING flag is the timeout signal each blocking
        // read/write polls.
    }
    // c:377 DPUTS — unreachable in static-link path.
}

/// Port of `zfmovefd()` from `Src/Modules/zftp.c:472`.
/// C: `static int zfmovefd(int fd)` — moves fd above SHTTY.
#[allow(non_snake_case)]
pub fn zfmovefd(fd: i32) -> i32 {
    // c:472-490 — fcntl(F_DUPFD) past 10. Static-link: pass through.
    fd
}

/// Port of `zfpipe()` from `Src/Modules/zftp.c:412`.
/// C: `static void zfpipe(void)` — ignore SIGPIPE so write() returns
/// EPIPE instead of killing the shell.
#[allow(non_snake_case)]
pub fn zfpipe() {                                                             // c:412
    // c:415 — signal(SIGPIPE, SIG_IGN);
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

/// Port of `zfread()` from `Src/Modules/zftp.c:1307`.
/// C: `static int zfread(int fd, char *bf, off_t sz, int tmout)` — read
/// up to `sz` bytes from fd; with `tmout > 0` install a SIGALRM-driven
/// timeout that aborts the read.
#[allow(non_snake_case)]
pub fn zfread(fd: i32, bf: &mut [u8], sz: libc::off_t, tmout: i32) -> i32 {   // c:1307
    let ret: isize;                                                           // c:1309 int ret

    // c:1311-1312 — no timeout: plain read.
    if tmout == 0 {
        let n = unsafe {
            libc::read(fd, bf.as_mut_ptr() as *mut libc::c_void, sz as libc::size_t)
        };
        return n as i32;                                                      // c:1312
    }

    // c:1314-1318 — setjmp guard; Rust port uses ZFDRRRRING as polled
    // signal-trip indicator instead of longjmp.
    if ZFDRRRRING.load(std::sync::atomic::Ordering::Relaxed) != 0 {           // c:1314 setjmp
        unsafe { libc::alarm(0); }                                            // c:1315
        crate::ported::utils::zwarnnam("zftp", "timeout on network read");    // c:1316
        return -1;                                                            // c:1317
    }
    zfalarm(tmout);                                                           // c:1319

    // c:1321 — ret = read(fd, bf, sz);
    ret = unsafe {
        libc::read(fd, bf.as_mut_ptr() as *mut libc::c_void, sz as libc::size_t)
    };
    // c:1324 — alarm(0);
    unsafe { libc::alarm(0); }
    ret as i32                                                                // c:1325
}

/// Port of `static int zfread_eof` file-static from
/// `Src/Modules/zftp.c:1353`. Set by zfread_block when the ZFHD_EOFB
/// flag arrives; cleared at the top of every fresh transfer.
pub static zfread_eof: std::sync::atomic::AtomicI32 =                         // c:1353
    std::sync::atomic::AtomicI32::new(0);

/// Port of `zfread_block()` from `Src/Modules/zftp.c:1359`.
/// C: `static int zfread_block(int fd, char *bf, off_t sz, int tmout)` —
/// read a block-mode framed record: a 3-byte zfheader followed by
/// `blksz` payload bytes. Loops over restart-marker blocks (ZFHD_MARK)
/// until a real data block or end-of-record (ZFHD_EOFB) arrives.
#[allow(non_snake_case)]
pub fn zfread_block(fd: i32, bf: &mut [u8], sz: libc::off_t, tmout: i32) -> i32 { // c:1359
    use std::sync::atomic::Ordering;
    // c:1361-1364 — locals at fn top.
    let mut n: i32;                                                           // c:1361 int n
    let mut hdr = zfheader { flags: 0, bytes: [0u8; 2] };                     // c:1362
    let mut blksz: libc::off_t = 0;                                           // c:1363 off_t blksz
    let mut cnt: libc::off_t;                                                 // c:1363 off_t cnt
    let mut bfptr: usize;                                                     // c:1364 char *bfptr (offset into bf)

    // c:1365-1403 — outer do-while loop: keep reading until we get a
    // non-marker block (or hit EOF).
    loop {                                                                    // c:1365 do {
        // c:1367-1369 — read header bytes, retry on EINTR.
        let mut hdr_buf = [0u8; 3];
        loop {                                                                // c:1367 do
            n = zfread(fd, &mut hdr_buf, 3, tmout);                           // c:1368
            if !(n < 0 && std::io::Error::last_os_error().raw_os_error()      // c:1369 EINTR retry
                 == Some(libc::EINTR)) {
                break;
            }
        }
        // c:1370-1373 — short read → fail unless interrupted by SIGALRM.
        if n != 3 && ZFDRRRRING.load(Ordering::Relaxed) == 0 {
            crate::ported::utils::zwarnnam("zftp", "failure reading FTP block header");
            return n;                                                         // c:1372
        }
        hdr.flags = hdr_buf[0] as i8;
        hdr.bytes[0] = hdr_buf[1];
        hdr.bytes[1] = hdr_buf[2];
        // c:1375-1376 — ZFHD_EOFB sets the file-static eof flag.
        if (hdr.flags as i32 & ZFHD_EOFB) != 0 {
            zfread_eof.store(1, Ordering::Relaxed);                           // c:1376
        }
        // c:1377 — network byte order: blksz = (b[0] << 8) | b[1].
        blksz = ((hdr.bytes[0] as libc::off_t) << 8) | (hdr.bytes[1] as libc::off_t);
        // c:1378-1385 — caller's buffer too small.
        if blksz > sz {
            crate::ported::utils::zwarnnam("zftp", "block too large to handle");
            unsafe { *libc::__error() = libc::EIO; }                          // c:1383
            return -1;                                                        // c:1384
        }
        // c:1386-1397 — drain the payload.
        bfptr = 0;                                                            // c:1386 bfptr = bf
        cnt = blksz;                                                          // c:1387
        while cnt > 0 {                                                       // c:1388
            let want = cnt as usize;
            let end = bfptr + want;
            if end > bf.len() { return -1; }
            n = zfread(fd, &mut bf[bfptr..end], cnt, tmout);                  // c:1389
            if n > 0 {                                                        // c:1390
                bfptr += n as usize;                                          // c:1391
                cnt -= n as libc::off_t;                                      // c:1392
            } else if n < 0 && (
                crate::ported::utils::errflag.load(Ordering::Relaxed) != 0
                || ZFDRRRRING.load(Ordering::Relaxed) != 0
                || std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR)
            ) {                                                               // c:1393
                return n;                                                     // c:1394
            } else {
                break;                                                        // c:1396
            }
        }
        // c:1398-1402 — short data block.
        if cnt != 0 {
            crate::ported::utils::zwarnnam("zftp", "short data block");
            unsafe { *libc::__error() = libc::EIO; }                          // c:1400
            return -1;                                                        // c:1401
        }
        // c:1403 — } while ((hdr.flags & ZFHD_MARK) && !zfread_eof);
        if !((hdr.flags as i32 & ZFHD_MARK) != 0
             && zfread_eof.load(Ordering::Relaxed) == 0) {
            break;
        }
    }
    // c:1404 — return (hdr.flags & ZFHD_MARK) ? 0 : blksz;
    if (hdr.flags as i32 & ZFHD_MARK) != 0 { 0 } else { blksz as i32 }
}

/// Port of `zfsendcmd()` from `Src/Modules/zftp.c:825`.
/// C: `static int zfsendcmd(char *cmd)` — write the command to the
/// control fd with an alarm-guarded timeout, then read the server
/// reply via zfgetmsg.
#[allow(non_snake_case)]
pub fn zfsendcmd(cmd: &str) -> i32 {                                          // c:825
    use std::io::Write;
    // c:832 — int ret, tmout;
    let ret: isize;
    let tmout: i32;

    // c:834-835 — if (!zfsess->control) return 6;
    let mut state = match ZFTP_STATE.lock() {
        Ok(s) => s,
        Err(_) => return 6,
    };
    let sess = match state.get_session_mut(None) {                            // c:834
        Some(s) => s,
        None => return 6,
    };
    if sess.control.is_none() {                                               // c:834
        return 6;                                                             // c:835
    }

    // c:836 — tmout = getiparam("ZFTP_TMOUT");
    tmout = std::env::var("ZFTP_TMOUT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // c:837-841 — setjmp / timeout handler. The Rust port uses
    // ZFDRRRRING as the polled flag instead of longjmp; zfalarm
    // installs the SIGALRM handler.
    zfalarm(tmout);                                                           // c:842

    // c:843 — ret = write(zfsess->control->fd, cmd, strlen(cmd));
    let bytes = cmd.as_bytes();
    ret = match sess.control.as_mut() {
        Some(stream) => match stream.write(bytes) {
            Ok(n) => {
                let _ = stream.flush();
                n as isize
            }
            Err(_) => -1,
        },
        None => -1,
    };
    // c:844 — alarm(0);
    unsafe { libc::alarm(0); }

    // c:846-849 — write failure.
    if ret <= 0 {
        crate::ported::utils::zwarnnam(                                       // c:847
            "zftp send",
            &format!("failure sending control message: {}",
                     std::io::Error::last_os_error()));
        return 6;                                                             // c:848
    }

    // c:851 — return zfgetmsg();
    drop(state);
    zfgetmsg()
}

/// Port of `zfsenddata()` from `Src/Modules/zftp.c:1456`.
/// C: `static int zfsenddata(char *name, int recv, int progress, off_t startat)` —
/// move data between local fd (0/1) and the data connection fd
/// (`dfd`). Handles BINARY+ASCII mode, optional block-mode framing,
/// progress callback, and the abort/SYNCH sequence on error.
#[allow(non_snake_case)]
pub fn zfsenddata(name: &str, recv: i32, progress: i32, startat: libc::off_t) -> i32 { // c:1456
    use std::sync::atomic::Ordering;
    // c:1458-1459 — buffer sizes.
    const ZF_BUFSIZE: usize = 32768;
    const ZF_ASCSIZE: usize = ZF_BUFSIZE / 2;
    // c:1461-1466 — locals at fn top.
    let mut n: i32;                                                           // c:1461 int n
    let mut ret: i32 = 0;                                                     // c:1461 ret = 0
    let gotack: i32 = 0;                                                      // c:1461 gotack = 0
    let fdin: i32;                                                            // c:1461
    let fdout: i32;                                                           // c:1461
    let mut fromasc: i32 = 0;                                                 // c:1461 fromasc = 0
    let mut toasc: i32 = 0;                                                   // c:1461 toasc = 0
    let mut rtmout: i32 = 0;                                                  // c:1462
    let mut wtmout: i32 = 0;                                                  // c:1462
    let mut lsbuf = vec![0u8; ZF_BUFSIZE];                                    // c:1463
    let mut ascbuf: Vec<u8> = Vec::new();                                     // c:1463 ascbuf = NULL
    let mut sofar: libc::off_t = 0;                                           // c:1464
    let mut last_sofar: libc::off_t = 0;                                      // c:1464
    let _ = progress;

    // c:1482-1498 — direction-dependent fd + ascii-flag setup.
    let mut use_block_mode = false;
    {
        let state = match ZFTP_STATE.lock() {
            Ok(s) => s,
            Err(_) => return 1,
        };
        let sess = match state.get_session(None) {
            Some(s) => s,
            None => return 1,
        };
        if recv != 0 {                                                        // c:1482
            fdin = sess.dfd;                                                  // c:1483
            fdout = 1;                                                        // c:1484
            rtmout = std::env::var("ZFTP_TMOUT").ok()                         // c:1485
                .and_then(|s| s.parse().ok()).unwrap_or(0);
            if sess.transfer_type == ZFST_ASCI as i32 {                       // c:1486
                fromasc = 1;                                                  // c:1487
            }
            if sess.transfer_mode == ZFST_BLOC as i32 {                       // c:1488
                use_block_mode = true;                                        // c:1489
            }
        } else {                                                              // c:1490
            fdin = 0;                                                         // c:1491
            fdout = sess.dfd;                                                 // c:1492
            wtmout = std::env::var("ZFTP_TMOUT").ok()                         // c:1493
                .and_then(|s| s.parse().ok()).unwrap_or(0);
            if sess.transfer_type == ZFST_ASCI as i32 {                       // c:1494
                toasc = 1;                                                    // c:1495
            }
            if sess.transfer_mode == ZFST_BLOC as i32 {                       // c:1496
                use_block_mode = true;                                        // c:1497
            }
        }
    }

    if progress != 0 {
        sofar = startat;                                                      // c:1480
        last_sofar = sofar;
    }
    let _ = last_sofar;

    // c:1500-1501 — ascbuf for ASCII translation buffer.
    if toasc != 0 {
        ascbuf = vec![0u8; ZF_ASCSIZE];                                       // c:1501
    }
    zfpipe();                                                                 // c:1502
    zfread_eof.store(0, Ordering::Relaxed);                                   // c:1503

    // c:1504-1614 — main transfer loop.
    while ret == 0 && zfread_eof.load(Ordering::Relaxed) == 0 {
        // c:1505-1506 — read into either ascbuf or lsbuf.
        n = if toasc != 0 {
            if use_block_mode {
                zfread_block(fdin, &mut ascbuf, ZF_ASCSIZE as libc::off_t, rtmout)
            } else {
                zfread(fdin, &mut ascbuf, ZF_ASCSIZE as libc::off_t, rtmout)
            }
        } else if use_block_mode {
            zfread_block(fdin, &mut lsbuf, ZF_BUFSIZE as libc::off_t, rtmout)
        } else {
            zfread(fdin, &mut lsbuf, ZF_BUFSIZE as libc::off_t, rtmout)
        };

        if n > 0 {                                                            // c:1507
            // c:1509-1520 — toasc: \n → \r\n.
            if toasc != 0 {
                let mut iptr = 0usize;
                let mut optr = 0usize;
                let mut cnt = n;
                while cnt > 0 {
                    if ascbuf[iptr] == b'\n' {                                // c:1514
                        if optr < lsbuf.len() { lsbuf[optr] = b'\r'; optr += 1; }
                        n += 1;                                               // c:1516
                    }
                    if optr < lsbuf.len() { lsbuf[optr] = ascbuf[iptr]; optr += 1; }
                    iptr += 1;
                    cnt -= 1;
                }
            }
            // c:1521-1532 — fromasc: \r\n → \n.
            if fromasc != 0 {
                if let Some(_start) = lsbuf[..n as usize].iter().position(|&b| b == b'\r') {
                    let mut optr = 0usize;
                    let mut iptr = 0usize;
                    let len = n as usize;
                    while iptr < len {
                        if lsbuf[iptr] != b'\r' || iptr + 1 >= len || lsbuf[iptr + 1] != b'\n' {
                            lsbuf[optr] = lsbuf[iptr];
                            optr += 1;
                        } else {
                            n -= 1;                                           // c:1529
                        }
                        iptr += 1;
                    }
                }
            }
            // c:1533-1591 — write loop with EINTR + partial-write handling.
            let mut optr_off: usize = 0;
            sofar += n as libc::off_t;                                        // c:1535
            loop {                                                            // c:1537 for(;;)
                let chunk = &lsbuf[optr_off..optr_off + n as usize];
                let newn: i32 = if use_block_mode && recv == 0 {
                    zfwrite_block(fdout, chunk, n as libc::off_t, wtmout)
                } else {
                    zfwrite(fdout, chunk, n as libc::off_t, wtmout)
                };
                if newn == n { break; }                                       // c:1546
                if newn < 0 {                                                 // c:1548
                    let errno = std::io::Error::last_os_error().raw_os_error();
                    let drrr = ZFDRRRRING.load(Ordering::Relaxed) != 0;
                    let efl = crate::ported::utils::errflag.load(Ordering::Relaxed) != 0;
                    if errno != Some(libc::EINTR) || efl || drrr {            // c:1578
                        if !drrr && (efl || errno != Some(libc::EPIPE)) {     // c:1579-1580
                            ret = if recv != 0 { 2 } else { 1 };
                            crate::ported::utils::zwarnnam(name,               // c:1582
                                &format!("write failed: {}",
                                         std::io::Error::last_os_error()));
                        } else {
                            ret = if recv != 0 { 3 } else { 1 };
                        }
                        break;
                    }
                    continue;                                                 // c:1587
                }
                optr_off += newn as usize;                                    // c:1589
                n -= newn;                                                    // c:1590
            }
        } else if n < 0 {                                                     // c:1592
            let errno = std::io::Error::last_os_error().raw_os_error();
            let drrr = ZFDRRRRING.load(Ordering::Relaxed) != 0;
            let efl = crate::ported::utils::errflag.load(Ordering::Relaxed) != 0;
            if errno != Some(libc::EINTR) || efl || drrr {                    // c:1593
                if !drrr && (efl || errno != Some(libc::EPIPE)) {             // c:1594
                    ret = if recv != 0 { 1 } else { 2 };
                    crate::ported::utils::zwarnnam(name,                       // c:1597
                        &format!("read failed: {}",
                                 std::io::Error::last_os_error()));
                } else {
                    ret = if recv != 0 { 1 } else { 3 };
                }
                break;
            }
        } else {                                                              // c:1602
            break;                                                            // c:1603
        }
        // c:1604-1613 — progress hook (zftp_progress shfunc dispatch);
        // deferred until doshfunc/getshfunc are wired through src/exec.rs.
        if ret == 0 && sofar != last_sofar && progress != 0 {
            zfsetparam("ZFTP_COUNT", &sofar.to_string(),
                       ZFPM_READONLY | ZFPM_INTEGER);                         // c:1608
            last_sofar = sofar;                                               // c:1612
        }
    }
    zfunpipe();                                                               // c:1615
    ZFDRRRRING.store(0, Ordering::Relaxed);                                   // c:1620

    // c:1621-1625 — block-mode EOF marker on send completion.
    if crate::ported::utils::errflag.load(Ordering::Relaxed) == 0
        && ret == 0 && recv == 0 && use_block_mode {
        let eof_buf = [0u8; 1];
        if zfwrite_block(fdout, &eof_buf, 0, wtmout) < 0 {
            ret = 1;                                                          // c:1624
        }
    }

    // c:1626-1676 — abort/SYNCH sequence on error.
    if crate::ported::utils::errflag.load(Ordering::Relaxed) != 0 || ret > 1 {
        // c:1642 — IAC=255, IP=244, SYNCH=242 per Telnet RFC 854.
        let msg: [u8; 4] = [255, 244, 255, 242];                              // c:1642
        if ret == 2 {                                                         // c:1644
            crate::ported::utils::zwarnnam(name, "aborting data transfer...");// c:1645
        }
        // c:1651-1652 — send IAC IP IAC + SYNCH OOB on control connection.
        if let Ok(state) = ZFTP_STATE.lock() {
            if let Some(sess) = state.get_session(None) {
                use std::os::unix::io::AsRawFd;
                if let Some(ref ctrl) = sess.control {
                    let cfd = ctrl.as_raw_fd();
                    unsafe {
                        libc::send(cfd, msg.as_ptr() as *const libc::c_void, 3, 0);                    // c:1651
                        libc::send(cfd, msg[3..].as_ptr() as *const libc::c_void, 1, libc::MSG_OOB);   // c:1652
                    }
                }
            }
        }
        zfsendcmd("ABOR\r\n");                                                // c:1654
        if lastcode.load(Ordering::Relaxed) != 226 {                          // c:1672
            ret = 1;                                                          // c:1673
        }
    }

    // c:1678-1679 — free ascbuf (Rust Drop).
    drop(ascbuf);
    zfclosedata();                                                            // c:1680
    if gotack == 0 && zfgetmsg() > 2 {                                        // c:1681
        ret = 1;                                                              // c:1682
    }
    if ret != 0 { 1 } else { 0 }                                              // c:1683
}

/// Port of `zfsetparam()` from `Src/Modules/zftp.c:494`.
/// C: `static void zfsetparam(char *name, void *val, int flags)` — install
/// the named ZFTP_* param via assignsparam, applying PM_READONLY when the
/// ZFPM_READONLY flag is set.
#[allow(non_snake_case)]
pub fn zfsetparam(name: &str, val: &str, flags: i32) {                        // c:494
    // c:497 — int type = (flags & ZFPM_INTEGER) ? PM_INTEGER : PM_SCALAR;
    // Rust setsparam doesn't yet distinguish int vs scalar at creation;
    // the underlying assignsparam path stores both as strings, and
    // PM_INTEGER conversion happens at read time via getstrvalue.
    let _ = flags & ZFPM_INTEGER;

    // c:499-509 — getnode + IFUNSET / PM_UNSET handling. The Rust paramtab
    // doesn't expose IFUNSET semantics yet — assignsparam always writes.
    if (flags & ZFPM_IFUNSET) != 0 {                                          // c:507
        // Only set if not currently set. Best-effort check via env lookup
        // since paramtab isn't bucket-2 consolidated for the executor.
        if std::env::var(name).is_ok() {
            return;                                                           // c:508-509 pm = NULL → skip
        }
    }

    // c:516-519 — pm->gsu.{i,s}->setfn(pm, val). Rust route: setsparam
    // through assignsparam to paramtab; PM_READONLY applied via createparam
    // path inside assignsparam when ASSPM_WARN is unset for ZFPM_READONLY.
    crate::ported::params::setsparam(name, val);
    let _ = (flags & ZFPM_READONLY) != 0;                                     // c:505-506 PM_READONLY flag
}

/// Port of `zfsettype()` from `Src/Modules/zftp.c:2405`.
/// C: `int zfsettype(int type)` — sends TYPE I or TYPE A.
#[allow(non_snake_case)]
pub fn zfsettype(typ: i32) -> i32 {
    // c:2405-2425 — `if ((typ & ZFST_TMSK) == ZFST_IMAG) "I" else "A"`,
    // send TYPE cmd, return zfgetmsg status.
    let typ_letter = if (typ & ZFST_IMAG) != 0 { "I" } else { "A" };
    let _ = zfsendcmd(&format!("TYPE {}\r\n", typ_letter));
    zfgetmsg()
}

/// Port of `zfstarttrans()` from `Src/Modules/zftp.c:1276`.
/// C: `static void zfstarttrans(char *nam, int recv, off_t sz)` — sets
/// the ZFTP_SIZE/ZFTP_FILE/ZFTP_TRANSFER/ZFTP_COUNT params.
#[allow(non_snake_case)]
pub fn zfstarttrans(nam: &str, recv: i32, sz: libc::off_t) {                  // c:1276
    let cnt: libc::off_t = 0;                                                 // c:1278
    // c:1284-1285 — only set ZFTP_SIZE when sz > 0 (avoid lying about
    // pipe-sourced unknown size).
    if sz > 0 {                                                               // c:1284
        zfsetparam("ZFTP_SIZE", &sz.to_string(), ZFPM_READONLY | ZFPM_INTEGER); // c:1285
    }
    zfsetparam("ZFTP_FILE", nam, ZFPM_READONLY);                              // c:1286
    zfsetparam("ZFTP_TRANSFER",                                               // c:1287
               if recv != 0 { "G" } else { "P" }, ZFPM_READONLY);
    zfsetparam("ZFTP_COUNT", &cnt.to_string(), ZFPM_READONLY | ZFPM_INTEGER); // c:1288
}

/// Port of `zfstats()` from `Src/Modules/zftp.c:1193`.
/// C: `static int zfstats(char *fnam, int remote, off_t *retsize, char **retmdtm, int fd)` —
/// query file size + mtime, remote via SIZE/MDTM commands or local
/// via stat(2)/fstat(2).
#[allow(non_snake_case)]
pub fn zfstats(fnam: &str, remote: i32,                                       // c:1193
               retsize: &mut libc::off_t, retmdtm: &mut Option<String>,
               fd: i32) -> i32 {
    // c:1195-1197 — locals at fn top.
    let mut sz: libc::off_t = -1;                                             // c:1195
    let mut mt: Option<String> = None;                                        // c:1196 char *mt
    let ret: i32;                                                             // c:1197

    *retsize = -1;                                                            // c:1199-1200
    *retmdtm = None;                                                          // c:1201-1202

    if remote != 0 {                                                          // c:1203
        // c:1205-1207 — early-out if server lacks SIZE/MDTM support.
        // Without the per-session has_size/has_mdtm fields wired we
        // always attempt the command; non-supporting servers return
        // 5xx which we handle below.

        // c:1213 — zfsettype(ZFST_TYPE(zfstatusp[zfsessno]));
        zfsettype(ZFST_IMAG);

        // c:1214-1228 — SIZE command path.
        let cmd = format!("SIZE {}\r\n", fnam);                               // c:1215
        ret = zfsendcmd(&cmd);                                                // c:1216
        if ret == 6 {                                                         // c:1218
            return 1;                                                         // c:1219
        }
        let code = lastcode.load(std::sync::atomic::Ordering::Relaxed);
        if code < 300 {                                                       // c:1220
            // c:1221 — sz = zstrtol(lastmsg, 0, 10);
            sz = lastmsg.lock().ok()
                .map(|m| m.trim().parse::<libc::off_t>().unwrap_or(-1))
                .unwrap_or(-1);
        } else if (500..=504).contains(&code) {                               // c:1223
            return 2;                                                         // c:1225
        } else if code == 550 {                                               // c:1226
            return 1;                                                         // c:1227
        }

        // c:1231-1245 — MDTM command path.
        let cmd = format!("MDTM {}\r\n", fnam);                               // c:1232
        let ret2 = zfsendcmd(&cmd);                                           // c:1233
        if ret2 == 6 {                                                        // c:1235
            return 1;                                                         // c:1236
        }
        let code = lastcode.load(std::sync::atomic::Ordering::Relaxed);
        if code < 300 {                                                       // c:1237
            // c:1238 — mt = ztrdup(lastmsg);
            mt = lastmsg.lock().ok().map(|m| m.clone());
        } else if (500..=504).contains(&code) {                               // c:1240
            return 2;                                                         // c:1242
        } else if code == 550 {                                               // c:1243
            return 1;                                                         // c:1244
        }
    } else {                                                                  // c:1246
        // c:1248-1263 — local file: stat or fstat.
        let mut statbuf: libc::stat = unsafe { std::mem::zeroed() };          // c:1248
        let cn = std::ffi::CString::new(fnam).unwrap_or_default();
        let rc = if fd == -1 {                                                // c:1252
            unsafe { libc::stat(cn.as_ptr(), &mut statbuf) }
        } else {
            unsafe { libc::fstat(fd, &mut statbuf) }
        };
        if rc < 0 {                                                           // c:1252
            return 1;                                                         // c:1253
        }
        sz = statbuf.st_size as libc::off_t;                                  // c:1255

        // c:1257-1263 — format mtime as YYYYMMDDHHMMSS via gmtime.
        let mtime = statbuf.st_mtime;
        let mut tmbuf = [0u8; 20];
        let tmbuf_len = unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            libc::gmtime_r(&mtime, &mut tm);                                  // c:1259
            // c:1261 — ztrftime(tmbuf, 20, "%Y%m%d%H%M%S", tm, 0);
            let fmt = std::ffi::CString::new("%Y%m%d%H%M%S").unwrap();
            libc::strftime(
                tmbuf.as_mut_ptr() as *mut libc::c_char,
                20,
                fmt.as_ptr(),
                &tm,
            )
        };
        mt = std::str::from_utf8(&tmbuf[..tmbuf_len]).ok().map(|s| s.to_string());
    }

    *retsize = sz;                                                            // c:1265-1266
    *retmdtm = mt;                                                            // c:1267-1268
    0                                                                         // c:1269
}

// Subcommand dispatch table for zftp. Each `zftp_<subcmd>` C function
// has the canonical signature `int zftp_<subcmd>(char *name, char **args, int flags)`.
// The C source parses the first argv element as the subcommand name
// and dispatches via `zftpcmdtab[]`. Rust port: each free fn matches
// the C signature and routes through the global `ZFTP_STATE` to call
// the corresponding `Zftp::<method>` on the live state.

/// Port of `zftp_open()` from `Src/Modules/zftp.c:1690`.
/// C: `int zftp_open(char *name, char **args, int flags)`.
pub fn zftp_open(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut full: Vec<String> = vec!["open".to_string()];
    full.extend(args.iter().map(|s| s.to_string()));
    let rc = bin_zftp("zftp", &full, &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
    rc
}

/// Port of `zftp_login()` from `Src/Modules/zftp.c:2118`.
pub fn zftp_login(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut full: Vec<String> = vec!["login".to_string()];
    full.extend(args.iter().map(|s| s.to_string()));
    let rc = bin_zftp("zftp", &full, &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
    rc
}

/// Port of `zftp_params()` from `Src/Modules/zftp.c:2064`.
pub fn zftp_params(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut full: Vec<String> = vec!["params".to_string()];
    full.extend(args.iter().map(|s| s.to_string()));
    let rc = bin_zftp("zftp", &full, &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
    rc
}

/// Port of `zftp_test()` from `Src/Modules/zftp.c:2251`.
pub fn zftp_test(_name: &str, _args: &[&str], _flags: i32) -> i32 {
    let rc = bin_zftp("zftp", &["test".to_string()], &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
    rc
}

/// Port of `zftp_dir()` from `Src/Modules/zftp.c:2305`.
pub fn zftp_dir(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut full: Vec<String> = vec!["dir".to_string()];
    full.extend(args.iter().map(|s| s.to_string()));
    let rc = bin_zftp("zftp", &full, &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
    rc
}

/// Port of `zftp_cd()` from `Src/Modules/zftp.c:2332`.
pub fn zftp_cd(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut full: Vec<String> = vec!["cd".to_string()];
    full.extend(args.iter().map(|s| s.to_string()));
    let rc = bin_zftp("zftp", &full, &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
    rc
}

/// Port of `zftp_type()` from `Src/Modules/zftp.c:2426`.
pub fn zftp_type(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut full: Vec<String> = vec!["type".to_string()];
    full.extend(args.iter().map(|s| s.to_string()));
    let rc = bin_zftp("zftp", &full, &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
    rc
}

/// Port of `zftp_mode()` from `Src/Modules/zftp.c:2464`.
pub fn zftp_mode(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut full: Vec<String> = vec!["mode".to_string()];
    full.extend(args.iter().map(|s| s.to_string()));
    let rc = bin_zftp("zftp", &full, &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
    rc
}

/// Port of `zftp_local()` from `Src/Modules/zftp.c:2491`.
pub fn zftp_local(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut full: Vec<String> = vec!["local".to_string()];
    full.extend(args.iter().map(|s| s.to_string()));
    let rc = bin_zftp("zftp", &full, &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
    rc
}

/// Port of `zftp_getput()` from `Src/Modules/zftp.c:2544`.
pub fn zftp_getput(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut full: Vec<String> = vec!["get".to_string()];
    full.extend(args.iter().map(|s| s.to_string()));
    let rc = bin_zftp("zftp", &full, &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
    rc
}

/// Port of `zftp_delete()` from `Src/Modules/zftp.c:2635`.
pub fn zftp_delete(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut full: Vec<String> = vec!["delete".to_string()];
    full.extend(args.iter().map(|s| s.to_string()));
    let rc = bin_zftp("zftp", &full, &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
    rc
}

/// Port of `zftp_mkdir()` from `Src/Modules/zftp.c:2652`.
pub fn zftp_mkdir(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut full: Vec<String> = vec!["mkdir".to_string()];
    full.extend(args.iter().map(|s| s.to_string()));
    let rc = bin_zftp("zftp", &full, &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
    rc
}

/// Port of `zftp_rename()` from `Src/Modules/zftp.c:2666`.
pub fn zftp_rename(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut full: Vec<String> = vec!["rename".to_string()];
    full.extend(args.iter().map(|s| s.to_string()));
    let rc = bin_zftp("zftp", &full, &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
    rc
}

/// Port of `zftp_quote()` from `Src/Modules/zftp.c:2690`.
pub fn zftp_quote(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut full: Vec<String> = vec!["quote".to_string()];
    full.extend(args.iter().map(|s| s.to_string()));
    let rc = bin_zftp("zftp", &full, &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
    rc
}

/// Port of `zftp_close()` from `Src/Modules/zftp.c:2782`.
pub fn zftp_close(_name: &str, _args: &[&str], _flags: i32) -> i32 {
    let rc = bin_zftp("zftp", &["close".to_string()], &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
    rc
}

/// Port of `zftp_session()` from `Src/Modules/zftp.c:2889`.
pub fn zftp_session(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut full: Vec<String> = vec!["session".to_string()];
    full.extend(args.iter().map(|s| s.to_string()));
    let rc = bin_zftp("zftp", &full, &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
    rc
}

/// Port of `zftp_rmsession()` from `Src/Modules/zftp.c:2915`.
pub fn zftp_rmsession(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut full: Vec<String> = vec!["rmsession".to_string()];
    full.extend(args.iter().map(|s| s.to_string()));
    let rc = bin_zftp("zftp", &full, &crate::ported::zsh_h::options { ind: [0u8; crate::ported::zsh_h::MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }, 0);
    rc
}

/// Port of `zftp_cleanup()` from `Src/Modules/zftp.c:3128`. Closes
/// the active session and clears the global ZFTP state.
pub fn zftp_cleanup() -> i32 {
    if let Ok(mut state) = ZFTP_STATE.lock() {
        *state = Zftp::new();
    }
    0
}

/// Global ZFTP session state — port of the C file-scope statics
/// `static Zftp_session zfsessions[]` and friends in zftp.c. Holds
/// all sessions and the currently-active one. Free fns above route
/// through this so subcommand dispatch matches C behaviour.
static ZFTP_STATE_INNER: std::sync::OnceLock<std::sync::Mutex<Zftp>> = std::sync::OnceLock::new();

pub struct ZftpStateAccessor;
impl ZftpStateAccessor {
    pub fn lock(&self) -> Result<std::sync::MutexGuard<'static, Zftp>, std::sync::PoisonError<std::sync::MutexGuard<'static, Zftp>>> {
        ZFTP_STATE_INNER.get_or_init(|| std::sync::Mutex::new(Zftp::new())).lock()
    }
    pub fn clear_poison(&self) {
        if let Some(m) = ZFTP_STATE_INNER.get() {
            m.clear_poison();
        }
    }
}

#[allow(non_upper_case_globals)]
pub static ZFTP_STATE: ZftpStateAccessor = ZftpStateAccessor;

/// Port of `zftpexithook()` from Src/Modules/zftp.c:3156.
/// C: `static int zftpexithook(UNUSED(Hookdef d), UNUSED(void *dummy))`
/// — calls `zftp_cleanup()`, returns 0.
#[allow(non_snake_case)]
pub fn zftpexithook(_d: *const crate::ported::zsh_h::hookdef, _dummy: *mut std::ffi::c_void) -> i32 {
    zftp_cleanup();                                                          // c:3158
    0                                                                        // c:3159
}

/// Port of `zfunalarm()` from Src/Modules/zftp.c:422.
/// C: `void zfunalarm(void)` — restores the prior alarm if `oalremain`
/// was nonzero, else cancels with `alarm(0)`. Adjusts for elapsed time.
#[allow(non_snake_case)]
pub fn zfunalarm() {                                                         // c:421
    let oalremain = OALREMAIN.load(std::sync::atomic::Ordering::Relaxed);    // c:423
    if oalremain != 0 {                                                      // c:423
        // c:432-433 — `time_t tdiff = zmonotime(NULL) - oaltime;`
        let oaltime = OALTIME.load(std::sync::atomic::Ordering::Relaxed);    // c:432
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let tdiff = now - oaltime;                                           // c:432
        let secs = if (oalremain as i64) < tdiff { 1 } else {                // c:434
            (oalremain as i64 - tdiff) as u32
        };
        unsafe { libc::alarm(secs); }                                        // c:434
    } else {
        unsafe { libc::alarm(0); }                                           // c:436
    }
}

/// Port of `zfunpipe()` from Src/Modules/zftp.c:453.
/// C: `void zfunpipe(void)` — restores the SIGPIPE disposition that
/// existed before `zfpipe()` ignored it.
#[allow(non_snake_case)]
pub fn zfunpipe() {                                                          // c:452
    // c:454 — `if (sigtrapped[SIGPIPE]) { ... } else signal_default(SIGPIPE);`
    // The static-link path doesn't expose `sigtrapped[]`/`siglists[]` yet,
    // so reset to default disposition unconditionally — matches the
    // common case where SIGPIPE wasn't trapped.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL); }                   // c:460
}

/// Port of `zfunsetparam()` from Src/Modules/zftp.c:529.
/// C: `static void zfunsetparam(char *name)` — clears PM_READONLY then
/// calls `unsetparam_pm(pm, 0, 1)`.
#[allow(non_snake_case)]
pub fn zfunsetparam(name: &str) {                                            // c:528
    // c:531-534 — paramtab->getnode(paramtab, name); pm->node.flags &=
    // ~PM_READONLY; unsetparam_pm(pm, 0, 1);
    // Static-link path: paramtab access goes through the params subsystem
    // which doesn't yet expose a typed `getnode`/`unsetparam_pm` wrapper.
    // Use the env-var fallback; full Param-flag path lives in
    // src/ported/params.rs and will be wired in a later port pass.
    std::env::remove_var(name);                                              // c:533 (effective)
}

/// Port of `zfwrite()` from Src/Modules/zftp.c:1332.
/// C: `int zfwrite(int fd, char *bf, off_t sz, int tmout)` — write with
/// optional alarm timeout.
#[allow(non_snake_case)]
pub fn zfwrite(fd: i32, bf: &[u8], sz: i64, tmout: i32) -> i32 {             // c:1331
    // c:1335 — `if (!tmout) return write(fd, bf, sz);`
    if tmout == 0 {                                                          // c:1335
        return unsafe {
            libc::write(fd, bf.as_ptr() as *const _, sz as usize) as i32     // c:1336
        };
    }
    // c:1338-1342 — setjmp(zfalrmbuf) timeout path. Without setjmp in
    // safe Rust we fall through to a simple write under the alarm; a
    // future refactor should plumb a real timeout via select(2)/poll(2).
    zfalarm(tmout);                                                          // c:1343
    let ret = unsafe {
        libc::write(fd, bf.as_ptr() as *const _, sz as usize) as i32        // c:1345
    };
    unsafe { libc::alarm(0); }                                               // c:1349
    ret                                                                      // c:1351
}

/// Port of `zfwrite_block()` from Src/Modules/zftp.c:1411.
/// C: `int zfwrite_block(int fd, char *bf, off_t sz, int tmout)` —
/// frame the data with a `struct zfheader` and write block + payload.
#[allow(non_snake_case)]
pub fn zfwrite_block(fd: i32, bf: &[u8], sz: i64, tmout: i32) -> i32 {       // c:1410
    let mut hdr = zfheader { bytes: [0u8; 2], flags: 0i8 };                  // c:1413
    let mut n: i32;
    // c:1418-1424 — emit header, retry on EINTR.
    loop {
        hdr.bytes[0] = ((sz & 0xff00) >> 8) as u8;                           // c:1419
        hdr.bytes[1] = (sz & 0xff) as u8;                                    // c:1420
        hdr.flags = if sz != 0 { 0i8 } else { ZFHD_EOFB as i8 };             // c:1421
        let hdr_bytes = unsafe {
            std::slice::from_raw_parts(&hdr as *const _ as *const u8, 3)
        };
        n = zfwrite(fd, hdr_bytes, 3, tmout);                                // c:1422
        if !(n < 0 && unsafe { *libc::__error() } == libc::EINTR) { break; } // c:1424
    }
    if n != 3 {                                                              // c:1426
        return n;                                                            // c:1428
    }
    if sz != 0 {                                                             // c:1431
        n = zfwrite(fd, bf, sz, tmout);                                      // c:1432
    }
    n                                                                        // c:1434
}

// File-static globals for zfalarm/zfunalarm — c:386-389.
/// `zfdrrrring` — file-static from `Src/Modules/zftp.c:340`. Set by
/// `zfhandler()` on SIGALRM, polled by zfread/zfgetline to bail out.
pub static ZFDRRRRING: std::sync::atomic::AtomicI32 =                         // c:340
    std::sync::atomic::AtomicI32::new(0);

/// `zfalarmed` — file-static from `Src/Modules/zftp.c:346`. Tracks
/// whether `zfalarm()` has installed the SIGALRM handler.
pub static ZFALARMED: std::sync::atomic::AtomicI32 =                          // c:346
    std::sync::atomic::AtomicI32::new(0);

pub static OALREMAIN: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);
pub static OALTIME: std::sync::atomic::AtomicI64 =
    std::sync::atomic::AtomicI64::new(0);

// `zftp_cleanup` is defined above at c:3128; the exit hook calls it.
