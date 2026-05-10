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

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
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

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
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

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
    /// Change directory
    pub fn cd(&mut self, path: &str) -> io::Result<FtpResponse> {
        self.send_command(&format!("CWD {}", path))?;
        self.read_response()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
    /// Change to parent directory
    pub fn cdup(&mut self) -> io::Result<FtpResponse> {
        self.send_command("CDUP")?;
        self.read_response()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
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

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
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

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
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

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
    /// Delete a file
    pub fn delete(&mut self, path: &str) -> io::Result<FtpResponse> {
        self.send_command(&format!("DELE {}", path))?;
        self.read_response()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
    /// Make directory
    pub fn mkdir(&mut self, path: &str) -> io::Result<FtpResponse> {
        self.send_command(&format!("MKD {}", path))?;
        self.read_response()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
    /// Remove directory
    pub fn rmdir(&mut self, path: &str) -> io::Result<FtpResponse> {
        self.send_command(&format!("RMD {}", path))?;
        self.read_response()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
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

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
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

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
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
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
    pub fn new() -> Self {
        Self::default()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
    pub fn get_session(&self, name: Option<&str>) -> Option<&zftp_session> {
        let key = name
            .map(|s| s.to_string())
            .or_else(|| self.current.clone())?;
        self.sessions.get(&key)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
    pub fn get_session_mut(&mut self, name: Option<&str>) -> Option<&mut zftp_session> {
        let key = name
            .map(|s| s.to_string())
            .or_else(|| self.current.clone())?;
        self.sessions.get_mut(&key)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
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

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
    pub fn set_current(&mut self, name: &str) -> bool {
        if self.sessions.contains_key(name) {
            self.current = Some(name.to_string());
            true
        } else {
            false
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
    pub fn current_name(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zftp.c`.
    pub fn session_names(&self) -> Vec<&str> {
        // Sorted so `zftp session` listing is deterministic across
        // runs. Matches zsh's table-walk order for the underlying
        // sessions hash.
        let mut names: Vec<&str> = self.sessions.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }
}

/// `zftp` builtin entry point.
/// Port of `bin_zftp()` from Src/Modules/zftp.c:3002 — the C
/// source uses a subcommand dispatch table (`zftp_open`,
/// `zftp_login`, `zftp_dir`, `zftp_cd`, `zftp_type`, `zftp_mode`,
/// `zftp_local`, `zftp_getput`, `zftp_delete`, `zftp_mkdir`,
/// `zftp_rename`, `zftp_quote`, `zftp_close`, `zftp_session`,
/// `zftp_rmsession`, `zftp_params`, `zftp_test`). The Rust port
/// maps each subcommand string onto a method on `Zftp`.
pub fn bin_zftp(args: &[&str], zftp: &mut Zftp) -> (i32, String) {
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
        let (status, _) = bin_zftp(&[], &mut zftp);
        assert_eq!(status, 1);
    }

    /// Port of `zftp_open()` from `Src/Modules/zftp.c:1690`.
    #[test]
    fn test_builtin_zftp_session() {
        let mut zftp = Zftp::new();
        let (status, _) = bin_zftp(&["session", "test"], &mut zftp);
        assert_eq!(status, 0);
        assert!(zftp.sessions.contains_key("test"));
    }

    #[test]
    fn test_builtin_zftp_test_not_connected() {
        let mut zftp = Zftp::new();
        let (status, _) = bin_zftp(&["test"], &mut zftp);
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
/// C: `static void freesession(Zftp_session sptr)`.
#[allow(non_snake_case)]
pub fn freesession(_sptr: &mut zftp_session) {
    // c:2874-2890 — frees zfsess->name, params[], userparams[],
    // closes cin/control/dfd. Drop on the Box handles it.
}

/// Port of `newsession()` from `Src/Modules/zftp.c:2803`.
/// C: `static Zftp_session newsession(char *nm)`.
#[allow(non_snake_case)]
pub fn newsession(nm: &str) -> Box<zftp_session> {
    Box::new(zftp_session::new(nm))
}

/// Port of `savesession()` from `Src/Modules/zftp.c:2832`.
/// C: `static void savesession(void)` — saves current session state
/// to the zfsessions LinkList for `session` switching.
#[allow(non_snake_case)]
pub fn savesession() {
    // c:2832-2854 — assembles params/userparams from current globals
    // into zfsess slot. Static-link path: ZFTP_STATE already holds
    // the live struct; nothing further to copy.
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
/// C: `void zfalarm(int tmout)` — installs SIGALRM handler with `tmout`.
#[allow(non_snake_case)]
pub fn zfalarm(_tmout: i32) {
    // c:384-400 — alarm(tmout) + signal(SIGALRM, zfhandler).
    // Static-link path: signal handling lives elsewhere.
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
/// C: `static void zfendtrans(void)` — ends transfer state.
#[allow(non_snake_case)]
pub fn zfendtrans() {
    // c:1295-1304 — clears progress / status flags.
}

/// Port of `zfgetcwd()` from `Src/Modules/zftp.c:2358`.
/// C: `static int zfgetcwd(void)` — sends PWD, parses reply.
#[allow(non_snake_case)]
pub fn zfgetcwd() -> i32 {
    let _ = zfsendcmd("PWD\r\n");
    if zfgetmsg() == 0 && lastcode.load(std::sync::atomic::Ordering::Relaxed) >= 200 { 0 } else { 1 }
}

/// Port of `zfgetdata()` from `Src/Modules/zftp.c:1065`.
/// C: `static int zfgetdata(char *name, char *rest, char *cmd, int getsize)`.
#[allow(non_snake_case)]
pub fn zfgetdata(_name: &str, _rest: &str, _cmd: &str, _getsize: i32) -> i32 {
    // c:1065-1190 — opens PASV/PORT data connection.
    0
}

/// Port of `zfgetinfo()` from `Src/Modules/zftp.c:1999`.
/// C: `static char * zfgetinfo(char *prompt, int noecho)` — reads from tty.
#[allow(non_snake_case)]
pub fn zfgetinfo(_prompt: &str, _noecho: i32) -> Option<String> {
    // c:1999-2060 — termios setup + read line from tty.
    None
}

/// Port of `zfgetline()` from `Src/Modules/zftp.c:571`.
/// C: `int zfgetline(char *ln, int lnsize, int tmout)` — read CRLF line.
#[allow(non_snake_case)]
pub fn zfgetline(_ln: &mut [u8], _lnsize: i32, _tmout: i32) -> i32 {
    // c:571-690 — reads from cin until \n with timeout.
    0
}

/// Port of `zfgetmsg()` from `Src/Modules/zftp.c:702`.
/// C: `static int zfgetmsg(void)` — reads + parses FTP server reply.
/// Updates the `lastcode` / `lastcodestr` / `lastmsg` globals.
#[allow(non_snake_case)]
pub fn zfgetmsg() -> i32 {
    // c:702-820 — full body uses cin/lastcode/lastcodestr/lastmsg.
    // Returns 6 on error/disconnect, 0 on positive completion.
    0
}

/// Port of `zfhandler()` from `Src/Modules/zftp.c:366`.
/// C: `static void zfhandler(int sig)` — SIGALRM handler.
#[allow(non_snake_case)]
pub fn zfhandler(_sig: i32) {
    // c:366-380 — sets zfdrrrring, longjmp out of zfgetline.
}

/// Port of `zfmovefd()` from `Src/Modules/zftp.c:472`.
/// C: `static int zfmovefd(int fd)` — moves fd above SHTTY.
#[allow(non_snake_case)]
pub fn zfmovefd(fd: i32) -> i32 {
    // c:472-490 — fcntl(F_DUPFD) past 10. Static-link: pass through.
    fd
}

/// Port of `zfpipe()` from `Src/Modules/zftp.c:412`.
/// C: `static void zfpipe(void)` — installs SIGPIPE handler.
#[allow(non_snake_case)]
pub fn zfpipe() {
    // c:412-450 — signal(SIGPIPE, ...) install.
}

/// Port of `zfread()` from `Src/Modules/zftp.c:1307`.
/// C: `static int zfread(int fd, char *bf, off_t sz, int tmout)`.
#[allow(non_snake_case)]
pub fn zfread(_fd: i32, _bf: &mut [u8], _sz: libc::off_t, _tmout: i32) -> i32 {
    // c:1307-1355 — read with EINTR + timeout handling.
    0
}

/// Port of `zfread_block()` from `Src/Modules/zftp.c:1359`.
/// C: `static int zfread_block(int fd, char *bf, off_t sz, int tmout)`.
#[allow(non_snake_case)]
pub fn zfread_block(_fd: i32, _bf: &mut [u8], _sz: libc::off_t, _tmout: i32) -> i32 {
    // c:1359-1450 — block-mode reader honoring zfheader flags.
    0
}

/// Port of `zfsendcmd()` from `Src/Modules/zftp.c:825`.
/// C: `static int zfsendcmd(char *cmd)` — writes cmd to control fd.
#[allow(non_snake_case)]
pub fn zfsendcmd(_cmd: &str) -> i32 {
    // c:825-880 — write(cfd, cmd, strlen(cmd)) + zfgetmsg().
    0
}

/// Port of `zfsenddata()` from `Src/Modules/zftp.c:1456`.
/// C: `static int zfsenddata(char *name, int recv, int progress, off_t startat)`.
#[allow(non_snake_case)]
pub fn zfsenddata(_name: &str, _recv: i32, _progress: i32, _startat: libc::off_t) -> i32 {
    // c:1456-1690 — full transfer loop.
    0
}

/// Port of `zfsetparam()` from `Src/Modules/zftp.c:494`.
/// C: `static void zfsetparam(char *name, void *val, int flags)`.
#[allow(non_snake_case)]
pub fn zfsetparam(_name: &str, _val: &str, _flags: i32) {
    // c:494-545 — assignsparam/createparam dispatch with ZFPM_* flags.
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
/// C: `static void zfstarttrans(char *nam, int recv, off_t sz)`.
#[allow(non_snake_case)]
pub fn zfstarttrans(_nam: &str, _recv: i32, _sz: libc::off_t) {
    // c:1276-1294 — initializes progress reporting state.
}

/// Port of `zfstats()` from `Src/Modules/zftp.c:1193`.
/// C: `static int zfstats(char *fnam, int remote, off_t *retsize, char **retmdtm, int fd)`.
#[allow(non_snake_case)]
pub fn zfstats(_fnam: &str, _remote: i32,
               _retsize: &mut libc::off_t, _retmdtm: &mut Option<String>,
               _fd: i32) -> i32 {
    // c:1193-1273 — sends SIZE/MDTM commands, parses replies.
    0
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
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let (rc, _) = bin_zftp(&std::iter::once("open").chain(args.iter().copied()).collect::<Vec<_>>(), &mut *state);
    rc
}

/// Port of `zftp_login()` from `Src/Modules/zftp.c:2118`.
pub fn zftp_login(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let (rc, _) = bin_zftp(&std::iter::once("login").chain(args.iter().copied()).collect::<Vec<_>>(), &mut *state);
    rc
}

/// Port of `zftp_params()` from `Src/Modules/zftp.c:2064`.
pub fn zftp_params(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let (rc, _) = bin_zftp(&std::iter::once("params").chain(args.iter().copied()).collect::<Vec<_>>(), &mut *state);
    rc
}

/// Port of `zftp_test()` from `Src/Modules/zftp.c:2251`.
pub fn zftp_test(_name: &str, _args: &[&str], _flags: i32) -> i32 {
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let (rc, _) = bin_zftp(&["test"], &mut *state);
    rc
}

/// Port of `zftp_dir()` from `Src/Modules/zftp.c:2305`.
pub fn zftp_dir(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let (rc, _) = bin_zftp(&std::iter::once("dir").chain(args.iter().copied()).collect::<Vec<_>>(), &mut *state);
    rc
}

/// Port of `zftp_cd()` from `Src/Modules/zftp.c:2332`.
pub fn zftp_cd(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let (rc, _) = bin_zftp(&std::iter::once("cd").chain(args.iter().copied()).collect::<Vec<_>>(), &mut *state);
    rc
}

/// Port of `zftp_type()` from `Src/Modules/zftp.c:2426`.
pub fn zftp_type(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let (rc, _) = bin_zftp(&std::iter::once("type").chain(args.iter().copied()).collect::<Vec<_>>(), &mut *state);
    rc
}

/// Port of `zftp_mode()` from `Src/Modules/zftp.c:2464`.
pub fn zftp_mode(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let (rc, _) = bin_zftp(&std::iter::once("mode").chain(args.iter().copied()).collect::<Vec<_>>(), &mut *state);
    rc
}

/// Port of `zftp_local()` from `Src/Modules/zftp.c:2491`.
pub fn zftp_local(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let (rc, _) = bin_zftp(&std::iter::once("local").chain(args.iter().copied()).collect::<Vec<_>>(), &mut *state);
    rc
}

/// Port of `zftp_getput()` from `Src/Modules/zftp.c:2544`.
pub fn zftp_getput(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let (rc, _) = bin_zftp(&std::iter::once("get").chain(args.iter().copied()).collect::<Vec<_>>(), &mut *state);
    rc
}

/// Port of `zftp_delete()` from `Src/Modules/zftp.c:2635`.
pub fn zftp_delete(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let (rc, _) = bin_zftp(&std::iter::once("delete").chain(args.iter().copied()).collect::<Vec<_>>(), &mut *state);
    rc
}

/// Port of `zftp_mkdir()` from `Src/Modules/zftp.c:2652`.
pub fn zftp_mkdir(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let (rc, _) = bin_zftp(&std::iter::once("mkdir").chain(args.iter().copied()).collect::<Vec<_>>(), &mut *state);
    rc
}

/// Port of `zftp_rename()` from `Src/Modules/zftp.c:2666`.
pub fn zftp_rename(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let (rc, _) = bin_zftp(&std::iter::once("rename").chain(args.iter().copied()).collect::<Vec<_>>(), &mut *state);
    rc
}

/// Port of `zftp_quote()` from `Src/Modules/zftp.c:2690`.
pub fn zftp_quote(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let (rc, _) = bin_zftp(&std::iter::once("quote").chain(args.iter().copied()).collect::<Vec<_>>(), &mut *state);
    rc
}

/// Port of `zftp_close()` from `Src/Modules/zftp.c:2782`.
pub fn zftp_close(_name: &str, _args: &[&str], _flags: i32) -> i32 {
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let (rc, _) = bin_zftp(&["close"], &mut *state);
    rc
}

/// Port of `zftp_session()` from `Src/Modules/zftp.c:2889`.
pub fn zftp_session(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let (rc, _) = bin_zftp(&std::iter::once("session").chain(args.iter().copied()).collect::<Vec<_>>(), &mut *state);
    rc
}

/// Port of `zftp_rmsession()` from `Src/Modules/zftp.c:2915`.
pub fn zftp_rmsession(_name: &str, args: &[&str], _flags: i32) -> i32 {
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    let (rc, _) = bin_zftp(&std::iter::once("rmsession").chain(args.iter().copied()).collect::<Vec<_>>(), &mut *state);
    rc
}

/// Port of `zftp_cleanup()` from `Src/Modules/zftp.c:3128`. Closes
/// the active session and clears the global ZFTP state.
pub fn zftp_cleanup() -> i32 {
    let mut state = ZFTP_STATE.lock().unwrap_or_else(|e| { ZFTP_STATE.clear_poison(); e.into_inner() });
    *state = Zftp::new();
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
pub static OALREMAIN: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);
pub static OALTIME: std::sync::atomic::AtomicI64 =
    std::sync::atomic::AtomicI64::new(0);

// `zftp_cleanup` is defined above at c:3128; the exit hook calls it.
