//! ZFTP module - port of Modules/zftp.c
//!
//! Provides a builtin FTP client for zsh.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

/// FTP transfer type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferType {
    Ascii,
    Binary,
}

impl TransferType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransferType::Ascii => "A",
            TransferType::Binary => "I",
        }
    }
}

/// FTP transfer mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferMode {
    Stream,
    Block,
}

impl TransferMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransferMode::Stream => "S",
            TransferMode::Block => "B",
        }
    }
}

/// FTP response
#[derive(Debug, Clone)]
pub struct FtpResponse {
    pub code: u32,
    pub message: String,
}

impl FtpResponse {
    pub fn is_positive(&self) -> bool {
        self.code >= 100 && self.code < 400
    }

    pub fn is_positive_completion(&self) -> bool {
        self.code >= 200 && self.code < 300
    }

    pub fn is_positive_intermediate(&self) -> bool {
        self.code >= 300 && self.code < 400
    }

    pub fn is_negative(&self) -> bool {
        self.code >= 400
    }
}

/// FTP session state
#[derive(Debug)]
pub struct FtpSession {
    pub name: String,
    pub host: Option<String>,
    pub port: u16,
    pub user: Option<String>,
    pub pwd: Option<String>,
    pub connected: bool,
    pub logged_in: bool,
    pub transfer_type: TransferType,
    pub transfer_mode: TransferMode,
    pub passive: bool,
    stream: Option<TcpStream>,
}

impl FtpSession {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            host: None,
            port: 21,
            user: None,
            pwd: None,
            connected: false,
            logged_in: false,
            transfer_type: TransferType::Binary,
            transfer_mode: TransferMode::Stream,
            passive: true,
            stream: None,
        }
    }

    fn send_command(&mut self, cmd: &str) -> io::Result<()> {
        if let Some(ref mut stream) = self.stream {
            write!(stream, "{}\r\n", cmd)?;
            stream.flush()
        } else {
            Err(io::Error::new(io::ErrorKind::NotConnected, "not connected"))
        }
    }

    fn read_response(&mut self) -> io::Result<FtpResponse> {
        let stream = self
            .stream
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

        Ok(FtpResponse {
            code,
            message: full_message,
        })
    }

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
            .spawn(move || { let _ = tx.send(dns.to_socket_addrs().map(|a| a.collect::<Vec<_>>())); })
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        let addrs = rx.recv_timeout(dns_timeout)
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "DNS resolution timed out"))?
            .map_err(|e| { tracing::warn!(host, error = %e, "zftp: DNS failed"); e })?;

        let sock_addr = addrs.into_iter().next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid address"))?;

        let stream = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(30))?;

        stream.set_read_timeout(Some(Duration::from_secs(60)))?;
        stream.set_write_timeout(Some(Duration::from_secs(60)))?;

        self.stream = Some(stream);
        self.host = Some(host.to_string());
        self.port = port;
        self.connected = true;

        self.read_response()
    }

    /// Login to FTP server
    pub fn login(&mut self, user: &str, pass: Option<&str>) -> io::Result<FtpResponse> {
        self.send_command(&format!("USER {}", user))?;
        let resp = self.read_response()?;

        if resp.code == 331 {
            let password = pass.unwrap_or("");
            self.send_command(&format!("PASS {}", password))?;
            let resp = self.read_response()?;

            if resp.is_positive_completion() {
                self.logged_in = true;
                self.user = Some(user.to_string());
            }
            return Ok(resp);
        }

        if resp.is_positive_completion() {
            self.logged_in = true;
            self.user = Some(user.to_string());
        }

        Ok(resp)
    }

    /// Set transfer type
    pub fn set_type(&mut self, transfer_type: TransferType) -> io::Result<FtpResponse> {
        self.send_command(&format!("TYPE {}", transfer_type.as_str()))?;
        let resp = self.read_response()?;
        if resp.is_positive_completion() {
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

        let pwd = if resp.is_positive_completion() {
            if let Some(start) = resp.message.find('"') {
                if let Some(end) = resp.message[start + 1..].find('"') {
                    Some(resp.message[start + 1..start + 1 + end].to_string())
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

        if !resp.is_positive() {
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

        if !resp.is_positive() {
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

    fn enter_passive_mode(&mut self) -> io::Result<TcpStream> {
        self.send_command("PASV")?;
        let resp = self.read_response()?;

        if !resp.is_positive_completion() {
            return Err(io::Error::new(io::ErrorKind::Other, resp.message));
        }

        let (ip, port) = parse_pasv_response(&resp.message)?;
        let addr = format!("{}:{}", ip, port);

        TcpStream::connect_timeout(
            &addr.to_socket_addrs()?.next().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "invalid PASV address")
            })?,
            Duration::from_secs(30),
        )
    }

    /// Download a file
    pub fn get(&mut self, remote: &str, local: &Path) -> io::Result<FtpResponse> {
        let mut data_stream = self.enter_passive_mode()?;

        self.send_command(&format!("RETR {}", remote))?;
        let resp = self.read_response()?;

        if !resp.is_positive() {
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

    /// Upload a file
    pub fn put(&mut self, local: &Path, remote: &str) -> io::Result<FtpResponse> {
        let mut data_stream = self.enter_passive_mode()?;

        self.send_command(&format!("STOR {}", remote))?;
        let resp = self.read_response()?;

        if !resp.is_positive() {
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

        if !resp.is_positive_intermediate() {
            return Ok(resp);
        }

        self.send_command(&format!("RNTO {}", to))?;
        self.read_response()
    }

    /// Get file size
    pub fn size(&mut self, path: &str) -> io::Result<(FtpResponse, Option<u64>)> {
        self.send_command(&format!("SIZE {}", path))?;
        let resp = self.read_response()?;

        let size = if resp.is_positive_completion() {
            resp.message
                .split_whitespace()
                .last()
                .and_then(|s| s.parse().ok())
        } else {
            None
        };

        Ok((resp, size))
    }

    /// Send raw command
    pub fn quote(&mut self, cmd: &str) -> io::Result<FtpResponse> {
        self.send_command(cmd)?;
        self.read_response()
    }

    /// Close connection
    pub fn close(&mut self) -> io::Result<FtpResponse> {
        if !self.connected {
            return Ok(FtpResponse {
                code: 0,
                message: "not connected".to_string(),
            });
        }

        let resp = if let Ok(()) = self.send_command("QUIT") {
            self.read_response().unwrap_or_else(|_| FtpResponse {
                code: 221,
                message: "Goodbye".to_string(),
            })
        } else {
            FtpResponse {
                code: 221,
                message: "Goodbye".to_string(),
            }
        };

        self.stream = None;
        self.connected = false;
        self.logged_in = false;
        self.host = None;
        self.user = None;
        self.pwd = None;

        Ok(resp)
    }
}

fn parse_pasv_response(msg: &str) -> io::Result<(String, u16)> {
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

/// FTP sessions manager
#[derive(Debug, Default)]
pub struct Zftp {
    sessions: HashMap<String, FtpSession>,
    current: Option<String>,
}

impl Zftp {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_session(&self, name: Option<&str>) -> Option<&FtpSession> {
        let key = name
            .map(|s| s.to_string())
            .or_else(|| self.current.clone())?;
        self.sessions.get(&key)
    }

    pub fn get_session_mut(&mut self, name: Option<&str>) -> Option<&mut FtpSession> {
        let key = name
            .map(|s| s.to_string())
            .or_else(|| self.current.clone())?;
        self.sessions.get_mut(&key)
    }

    pub fn create_session(&mut self, name: &str) -> &mut FtpSession {
        self.sessions
            .entry(name.to_string())
            .or_insert_with(|| FtpSession::new(name))
    }

    pub fn remove_session(&mut self, name: &str) -> Option<FtpSession> {
        let sess = self.sessions.remove(name);
        if self.current.as_deref() == Some(name) {
            self.current = self.sessions.keys().next().cloned();
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
        self.sessions.keys().map(|s| s.as_str()).collect()
    }
}

/// Execute zftp builtin
pub fn builtin_zftp(args: &[&str], zftp: &mut Zftp) -> (i32, String) {
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
                    if resp.is_positive() {
                        zftp.set_current(&session_name);
                        (0, resp.message)
                    } else {
                        (1, resp.message)
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
            let pass = args.get(2).map(|s| *s);

            let sess = match zftp.get_session_mut(None) {
                Some(s) => s,
                None => return (1, "zftp login: not connected\n".to_string()),
            };

            match sess.login(user, pass) {
                Ok(resp) => {
                    if resp.is_positive_completion() {
                        (0, resp.message)
                    } else {
                        (1, resp.message)
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
                    if resp.is_positive_completion() {
                        (0, resp.message)
                    } else {
                        (1, resp.message)
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
                    if resp.is_positive_completion() {
                        (0, resp.message)
                    } else {
                        (1, resp.message)
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
                        (1, resp.message)
                    }
                }
                Err(e) => (1, format!("zftp pwd: {}\n", e)),
            }
        }

        "dir" | "ls" => {
            let path = args.get(1).map(|s| *s);
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
                    if resp.is_positive_completion() {
                        (0, lines.join("\n") + "\n")
                    } else {
                        (1, resp.message)
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
                    if resp.is_positive_completion() {
                        (0, String::new())
                    } else {
                        (1, resp.message)
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
                    if resp.is_positive_completion() {
                        (0, String::new())
                    } else {
                        (1, resp.message)
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
                    if resp.is_positive_completion() {
                        (0, String::new())
                    } else {
                        (1, resp.message)
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
                    if resp.is_positive_completion() {
                        (0, String::new())
                    } else {
                        (1, resp.message)
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
                    if resp.is_positive_completion() {
                        (0, String::new())
                    } else {
                        (1, resp.message)
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
                    if resp.is_positive_completion() {
                        (0, String::new())
                    } else {
                        (1, resp.message)
                    }
                }
                Err(e) => (1, format!("zftp rename: {}\n", e)),
            }
        }

        "type" | "ascii" | "binary" => {
            let transfer_type = match args[0] {
                "ascii" => TransferType::Ascii,
                "binary" => TransferType::Binary,
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
                                if sess.transfer_type == TransferType::Ascii {
                                    "ascii"
                                } else {
                                    "binary"
                                }
                            ),
                        );
                    }
                    match args[1].to_lowercase().as_str() {
                        "a" | "ascii" => TransferType::Ascii,
                        "i" | "binary" | "image" => TransferType::Binary,
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
                    if resp.is_positive_completion() {
                        (0, String::new())
                    } else {
                        (1, resp.message)
                    }
                }
                Err(e) => (1, format!("zftp type: {}\n", e)),
            }
        }

        "quote" => {
            if args.len() < 2 {
                return (1, "zftp quote: command required\n".to_string());
            }

            let cmd = args[1..].join(" ");

            let sess = match zftp.get_session_mut(None) {
                Some(s) => s,
                None => return (1, "zftp quote: not connected\n".to_string()),
            };

            match sess.quote(&cmd) {
                Ok(resp) => (if resp.is_positive() { 0 } else { 1 }, resp.message),
                Err(e) => (1, format!("zftp quote: {}\n", e)),
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
        assert_eq!(TransferType::Ascii.as_str(), "A");
        assert_eq!(TransferType::Binary.as_str(), "I");
    }

    #[test]
    fn test_transfer_mode() {
        assert_eq!(TransferMode::Stream.as_str(), "S");
        assert_eq!(TransferMode::Block.as_str(), "B");
    }

    #[test]
    fn test_ftp_response_positive() {
        let resp = FtpResponse {
            code: 200,
            message: "OK".to_string(),
        };
        assert!(resp.is_positive());
        assert!(resp.is_positive_completion());
        assert!(!resp.is_negative());
    }

    #[test]
    fn test_ftp_response_intermediate() {
        let resp = FtpResponse {
            code: 331,
            message: "Password required".to_string(),
        };
        assert!(resp.is_positive());
        assert!(resp.is_positive_intermediate());
        assert!(!resp.is_positive_completion());
    }

    #[test]
    fn test_ftp_response_negative() {
        let resp = FtpResponse {
            code: 550,
            message: "File not found".to_string(),
        };
        assert!(resp.is_negative());
        assert!(!resp.is_positive());
    }

    #[test]
    fn test_ftp_session_new() {
        let sess = FtpSession::new("test");
        assert_eq!(sess.name, "test");
        assert!(!sess.connected);
        assert!(!sess.logged_in);
    }

    #[test]
    fn test_parse_pasv_response() {
        let msg = "227 Entering Passive Mode (192,168,1,1,4,1)";
        let (ip, port) = parse_pasv_response(msg).unwrap();
        assert_eq!(ip, "192.168.1.1");
        assert_eq!(port, 1025);
    }

    #[test]
    fn test_parse_pasv_response_invalid() {
        let msg = "invalid";
        assert!(parse_pasv_response(msg).is_err());
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
        let (status, _) = builtin_zftp(&[], &mut zftp);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_zftp_session() {
        let mut zftp = Zftp::new();
        let (status, _) = builtin_zftp(&["session", "test"], &mut zftp);
        assert_eq!(status, 0);
        assert!(zftp.sessions.contains_key("test"));
    }

    #[test]
    fn test_builtin_zftp_test_not_connected() {
        let mut zftp = Zftp::new();
        let (status, _) = builtin_zftp(&["test"], &mut zftp);
        assert_eq!(status, 1);
    }
}
