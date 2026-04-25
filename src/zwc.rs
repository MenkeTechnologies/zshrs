//! ZWC (Zsh Word Code) file parser
//!
//! Parses compiled zsh function files (.zwc) into function definitions
//! that can be executed by zshrs.

use crate::parser::{
    CaseTerminator, CompoundCommand, ListOp, Redirect, RedirectOp, ShellCommand, ShellWord,
    SimpleCommand,
};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

const FD_MAGIC: u32 = 0x04050607;
const FD_OMAGIC: u32 = 0x07060504; // Other byte order
const FD_PRELEN: usize = 12;

// Word code types (5 bits)
pub const WC_END: u32 = 0;
pub const WC_LIST: u32 = 1;
pub const WC_SUBLIST: u32 = 2;
pub const WC_PIPE: u32 = 3;
pub const WC_REDIR: u32 = 4;
pub const WC_ASSIGN: u32 = 5;
pub const WC_SIMPLE: u32 = 6;
pub const WC_TYPESET: u32 = 7;
pub const WC_SUBSH: u32 = 8;
pub const WC_CURSH: u32 = 9;
pub const WC_TIMED: u32 = 10;
pub const WC_FUNCDEF: u32 = 11;
pub const WC_FOR: u32 = 12;
pub const WC_SELECT: u32 = 13;
pub const WC_WHILE: u32 = 14;
pub const WC_REPEAT: u32 = 15;
pub const WC_CASE: u32 = 16;
pub const WC_IF: u32 = 17;
pub const WC_COND: u32 = 18;
pub const WC_ARITH: u32 = 19;
pub const WC_AUTOFN: u32 = 20;
pub const WC_TRY: u32 = 21;

// List flags
pub const Z_END: u32 = 1 << 4;
pub const Z_SIMPLE: u32 = 1 << 5;
pub const WC_LIST_FREE: u32 = 6;

// Sublist types
pub const WC_SUBLIST_END: u32 = 0;
pub const WC_SUBLIST_AND: u32 = 1;
pub const WC_SUBLIST_OR: u32 = 2;
pub const WC_SUBLIST_COPROC: u32 = 4;
pub const WC_SUBLIST_NOT: u32 = 8;
pub const WC_SUBLIST_SIMPLE: u32 = 16;
pub const WC_SUBLIST_FREE: u32 = 5;

// Pipe types
pub const WC_PIPE_END: u32 = 0;
pub const WC_PIPE_MID: u32 = 1;

// For types
pub const WC_FOR_PPARAM: u32 = 0;
pub const WC_FOR_LIST: u32 = 1;
pub const WC_FOR_COND: u32 = 2;

// While types
pub const WC_WHILE_WHILE: u32 = 0;
pub const WC_WHILE_UNTIL: u32 = 1;

// Case types
pub const WC_CASE_HEAD: u32 = 0;
pub const WC_CASE_OR: u32 = 1;
pub const WC_CASE_AND: u32 = 2;
pub const WC_CASE_TESTAND: u32 = 3;
pub const WC_CASE_FREE: u32 = 3;

// If types
pub const WC_IF_HEAD: u32 = 0;
pub const WC_IF_IF: u32 = 1;
pub const WC_IF_ELIF: u32 = 2;
pub const WC_IF_ELSE: u32 = 3;

pub const WC_CODEBITS: u32 = 5;

// Zsh tokens (from zsh.h)
const POUND: u8 = 0x84;
const STRING: u8 = 0x85; // $ for variables
const HAT: u8 = 0x86; // ^
const STAR: u8 = 0x87; // *
const INPAR: u8 = 0x88; // (
const OUTPAR: u8 = 0x8a; // )
const QSTRING: u8 = 0x8c; // $ in double quotes
const EQUALS: u8 = 0x8d; // =
const BAR: u8 = 0x8e; // |
const INBRACE: u8 = 0x8f; // {
const OUTBRACE: u8 = 0x90; // }
const INBRACK: u8 = 0x91; // [
const OUTBRACK: u8 = 0x92; // ]
const TICK: u8 = 0x93; // `
const INANG: u8 = 0x94; // <
const OUTANG: u8 = 0x95; // >
const QUEST: u8 = 0x97; // ?
const TILDE: u8 = 0x98; // ~
const COMMA: u8 = 0x9a; // ,
const SNULL: u8 = 0x9d; // ' quote marker
const DNULL: u8 = 0x9e; // " quote marker
const BNULL: u8 = 0x9f; // \ backslash marker
const NULARG: u8 = 0xa1; // empty argument marker

/// Untokenize a zsh tokenized string back to shell syntax
fn untokenize(bytes: &[u8]) -> String {
    let mut result = String::new();
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        match b {
            POUND => result.push('#'),
            STRING | QSTRING => result.push('$'),
            HAT => result.push('^'),
            STAR => result.push('*'),
            INPAR => result.push('('),
            OUTPAR => result.push(')'),
            EQUALS => result.push('='),
            BAR => result.push('|'),
            INBRACE => result.push('{'),
            OUTBRACE => result.push('}'),
            INBRACK => result.push('['),
            OUTBRACK => result.push(']'),
            TICK => result.push('`'),
            INANG => result.push('<'),
            OUTANG => result.push('>'),
            QUEST => result.push('?'),
            TILDE => result.push('~'),
            COMMA => result.push(','),
            SNULL | DNULL | BNULL | NULARG => {
                // Skip null markers
            }
            0x89 => result.push_str("(("), // Inparmath
            0x8b => result.push_str("))"), // Outparmath
            _ if b >= 0x80 => {
                // Unknown token, skip or try to represent
            }
            _ => result.push(b as char),
        }
        i += 1;
    }

    result
}

#[inline]
pub fn wc_code(c: u32) -> u32 {
    c & ((1 << WC_CODEBITS) - 1)
}

#[inline]
pub fn wc_data(c: u32) -> u32 {
    c >> WC_CODEBITS
}

#[derive(Debug)]
pub struct ZwcHeader {
    pub magic: u32,
    pub flags: u8,
    pub version: String,
    pub header_len: u32,
    pub other_offset: u32,
}

#[derive(Debug)]
pub struct ZwcFunction {
    pub name: String,
    pub start: u32,
    pub len: u32,
    pub npats: u32,
    pub strs_offset: u32,
    pub flags: u32,
}

#[derive(Debug)]
pub struct ZwcFile {
    pub header: ZwcHeader,
    pub functions: Vec<ZwcFunction>,
    pub wordcode: Vec<u32>,
    pub strings: Vec<u8>,
}

impl ZwcFile {
    pub fn load<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let mut buf = vec![0u8; (FD_PRELEN + 1) * 4];

        file.read_exact(&mut buf)?;

        let magic = u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]);

        let swap_bytes = if magic == FD_MAGIC {
            false
        } else if magic == FD_OMAGIC {
            true
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid ZWC magic: 0x{:08x}", magic),
            ));
        };

        let read_u32 = |bytes: &[u8], offset: usize| -> u32 {
            let b = &bytes[offset..offset + 4];
            let val = u32::from_ne_bytes([b[0], b[1], b[2], b[3]]);
            if swap_bytes {
                val.swap_bytes()
            } else {
                val
            }
        };

        let flags = buf[4];
        let other_offset = (buf[5] as u32) | ((buf[6] as u32) << 8) | ((buf[7] as u32) << 16);

        // Version string starts at offset 8 (word 2)
        let version_start = 8;
        let version_end = buf[version_start..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| version_start + p)
            .unwrap_or(buf.len());
        let version = String::from_utf8_lossy(&buf[version_start..version_end]).to_string();

        let header_len = read_u32(&buf, FD_PRELEN * 4);

        let header = ZwcHeader {
            magic,
            flags,
            version,
            header_len,
            other_offset,
        };

        // Read full header
        file.seek(SeekFrom::Start(0))?;
        let full_header_size = (header_len as usize) * 4;
        let mut header_buf = vec![0u8; full_header_size];
        file.read_exact(&mut header_buf)?;

        // Parse function headers (start after FD_PRELEN words)
        let mut functions = Vec::new();
        let mut offset = FD_PRELEN * 4;

        while offset < full_header_size {
            if offset + 24 > full_header_size {
                break;
            }

            let start = read_u32(&header_buf, offset);
            let len = read_u32(&header_buf, offset + 4);
            let npats = read_u32(&header_buf, offset + 8);
            let strs = read_u32(&header_buf, offset + 12);
            let hlen = read_u32(&header_buf, offset + 16);
            let flags = read_u32(&header_buf, offset + 20);

            // Name follows the header struct (6 words = 24 bytes)
            let name_start = offset + 24;
            let name_end = header_buf[name_start..]
                .iter()
                .position(|&b| b == 0)
                .map(|p| name_start + p)
                .unwrap_or(full_header_size);

            let name = String::from_utf8_lossy(&header_buf[name_start..name_end]).to_string();

            if name.is_empty() {
                break;
            }

            functions.push(ZwcFunction {
                name,
                start,
                len,
                npats,
                strs_offset: strs,
                flags,
            });

            // Move to next function header
            offset += (hlen as usize) * 4;
        }

        // Read the rest of the file (wordcode + strings)
        let mut rest = Vec::new();
        file.read_to_end(&mut rest)?;

        // Parse wordcode as u32 array
        let mut wordcode = Vec::new();
        let mut i = 0;
        while i + 4 <= rest.len() {
            let val = u32::from_ne_bytes([rest[i], rest[i + 1], rest[i + 2], rest[i + 3]]);
            wordcode.push(if swap_bytes { val.swap_bytes() } else { val });
            i += 4;
        }

        // Strings are embedded after wordcode for each function
        let strings = rest;

        Ok(ZwcFile {
            header,
            functions,
            wordcode,
            strings,
        })
    }

    pub fn list_functions(&self) -> Vec<&str> {
        self.functions.iter().map(|f| f.name.as_str()).collect()
    }

    pub fn function_count(&self) -> usize {
        self.functions.len()
    }

    /// Create a new empty ZWC file for building
    pub fn new_builder() -> ZwcBuilder {
        ZwcBuilder::new()
    }

    pub fn get_function(&self, name: &str) -> Option<&ZwcFunction> {
        self.functions
            .iter()
            .find(|f| f.name == name || f.name.ends_with(&format!("/{}", name)))
    }

    pub fn decode_function(&self, func: &ZwcFunction) -> Option<DecodedFunction> {
        let header_words = self.header.header_len as usize;
        let start_idx = (func.start as usize).saturating_sub(header_words);

        if start_idx >= self.wordcode.len() {
            return None;
        }

        // Strings are embedded at strs_offset bytes from the start of this function's wordcode
        // Convert byte offset to word offset to find where strings start
        let func_wordcode = &self.wordcode[start_idx..];

        // The strings are at byte offset strs_offset from the wordcode base
        // Create a string table from the wordcode bytes
        let mut string_bytes = Vec::new();
        for &wc in func_wordcode {
            string_bytes.extend_from_slice(&wc.to_ne_bytes());
        }

        let decoder = WordcodeDecoder::new(func_wordcode, &string_bytes, func.strs_offset as usize);

        Some(DecodedFunction {
            name: func.name.clone(),
            body: decoder.decode(),
        })
    }
}

/// Builder for creating ZWC files
#[derive(Debug)]
pub struct ZwcBuilder {
    functions: Vec<(String, Vec<u8>)>, // (name, source code)
}

impl ZwcBuilder {
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
        }
    }

    /// Add a function from source code
    pub fn add_source(&mut self, name: &str, source: &str) {
        self.functions
            .push((name.to_string(), source.as_bytes().to_vec()));
    }

    /// Add a function from a file
    pub fn add_file(&mut self, path: &std::path::Path) -> io::Result<()> {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Invalid filename"))?;
        let source = std::fs::read(path)?;
        self.functions.push((name.to_string(), source));
        Ok(())
    }

    /// Write the ZWC file
    /// Note: This writes a simplified format that stores raw source code
    /// rather than compiled wordcode. The loader handles both formats.
    pub fn write<P: AsRef<std::path::Path>>(&self, path: P) -> io::Result<()> {
        use std::io::Write;

        let mut file = std::fs::File::create(path)?;

        // Write magic
        file.write_all(&FD_MAGIC.to_ne_bytes())?;

        // Write flags (0 = not mapped)
        file.write_all(&[0u8])?;

        // Write other offset placeholder (3 bytes)
        file.write_all(&[0u8; 3])?;

        // Write version string (padded to 4-byte boundary)
        let version = env!("CARGO_PKG_VERSION");
        let version_bytes = version.as_bytes();
        file.write_all(version_bytes)?;
        file.write_all(&[0u8])?; // null terminator
                                 // Pad to 4-byte boundary
        let padding = (4 - ((version_bytes.len() + 1) % 4)) % 4;
        file.write_all(&vec![0u8; padding])?;

        // Calculate header length (in words)
        let mut header_words = FD_PRELEN;
        for (name, _) in &self.functions {
            // 6 words for fdhead struct + name (padded)
            header_words += 6 + (name.len() + 1 + 3) / 4;
        }

        // Write header length
        file.write_all(&(header_words as u32).to_ne_bytes())?;

        // Track positions for function data
        let mut data_offset = header_words;
        let mut func_data: Vec<(u32, u32, Vec<u8>)> = Vec::new(); // (start, len, data)

        // Write function headers
        for (name, source) in &self.functions {
            let source_words = (source.len() + 3) / 4;

            // fdhead: start, len, npats, strs, hlen, flags
            file.write_all(&(data_offset as u32).to_ne_bytes())?; // start
            file.write_all(&(source.len() as u32).to_ne_bytes())?; // len (in bytes)
            file.write_all(&0u32.to_ne_bytes())?; // npats
            file.write_all(&0u32.to_ne_bytes())?; // strs offset
            let hlen = 6 + (name.len() + 1 + 3) / 4;
            file.write_all(&(hlen as u32).to_ne_bytes())?; // hlen
            file.write_all(&0u32.to_ne_bytes())?; // flags

            // Write name (null-terminated, padded)
            file.write_all(name.as_bytes())?;
            file.write_all(&[0u8])?;
            let name_padding = (4 - ((name.len() + 1) % 4)) % 4;
            file.write_all(&vec![0u8; name_padding])?;

            func_data.push((data_offset as u32, source.len() as u32, source.clone()));
            data_offset += source_words;
        }

        // Write function data (source code, padded to 4 bytes)
        for (_, _, data) in &func_data {
            file.write_all(data)?;
            let padding = (4 - (data.len() % 4)) % 4;
            file.write_all(&vec![0u8; padding])?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct DecodedFunction {
    pub name: String,
    pub body: Vec<DecodedOp>,
}

#[derive(Debug, Clone)]
pub enum DecodedOp {
    End,
    LineNo(u32),
    List {
        list_type: u32,
        is_end: bool,
        ops: Vec<DecodedOp>,
    },
    Sublist {
        sublist_type: u32,
        negated: bool,
        ops: Vec<DecodedOp>,
    },
    Pipe {
        lineno: u32,
        ops: Vec<DecodedOp>,
    },
    Redir {
        redir_type: u32,
        fd: i32,
        target: String,
        varid: Option<String>,
    },
    Assign {
        name: String,
        value: String,
    },
    AssignArray {
        name: String,
        values: Vec<String>,
    },
    Simple {
        args: Vec<String>,
    },
    Typeset {
        args: Vec<String>,
        assigns: Vec<DecodedOp>,
    },
    Subsh {
        ops: Vec<DecodedOp>,
    },
    Cursh {
        ops: Vec<DecodedOp>,
    },
    Timed {
        cmd: Option<Box<DecodedOp>>,
    },
    FuncDef {
        name: String,
        body: Vec<DecodedOp>,
    },
    For {
        var: String,
        list: Vec<String>,
        body: Vec<DecodedOp>,
    },
    ForCond {
        init: String,
        cond: String,
        step: String,
        body: Vec<DecodedOp>,
    },
    Select {
        var: String,
        list: Vec<String>,
        body: Vec<DecodedOp>,
    },
    While {
        cond: Vec<DecodedOp>,
        body: Vec<DecodedOp>,
        is_until: bool,
    },
    Repeat {
        count: String,
        body: Vec<DecodedOp>,
    },
    Case {
        word: String,
        cases: Vec<(String, Vec<DecodedOp>)>,
    },
    CaseItem {
        pattern: String,
        terminator: u32,
        body: Vec<DecodedOp>,
    },
    If {
        if_type: u32,
        conditions: Vec<(Vec<DecodedOp>, Vec<DecodedOp>)>,
        else_body: Option<Vec<DecodedOp>>,
    },
    Cond {
        cond_type: u32,
        args: Vec<String>,
    },
    Arith {
        expr: String,
    },
    AutoFn,
    Try {
        try_body: Vec<DecodedOp>,
        always_body: Vec<DecodedOp>,
    },
    Unknown {
        code: u32,
        data: u32,
    },
}

pub struct WordcodeDecoder<'a> {
    code: &'a [u32],
    strings: &'a [u8],
    strs_base: usize,
    pub pos: usize,
}

impl<'a> WordcodeDecoder<'a> {
    pub fn new(code: &'a [u32], strings: &'a [u8], strs_base: usize) -> Self {
        Self {
            code,
            strings,
            strs_base,
            pos: 0,
        }
    }

    pub fn at_end(&self) -> bool {
        self.pos >= self.code.len()
    }

    pub fn peek(&self) -> Option<u32> {
        self.code.get(self.pos).copied()
    }

    pub fn next(&mut self) -> Option<u32> {
        let val = self.code.get(self.pos).copied();
        if val.is_some() {
            self.pos += 1;
        }
        val
    }

    pub fn read_string(&mut self) -> String {
        let wc = self.next().unwrap_or(0);
        self.decode_string(wc)
    }

    pub fn decode_string(&self, wc: u32) -> String {
        // Zsh string encoding from ecrawstr():
        // - c == 6 || c == 7 -> empty string
        // - c & 2 (bit 1 set) -> short string, chars in bits 3-10, 11-18, 19-26
        // - otherwise -> long string at strs + (c >> 2)

        if wc == 6 || wc == 7 {
            return String::new();
        }

        if (wc & 2) != 0 {
            // Short string (1-3 chars packed in upper bits)
            let mut s = String::new();
            let c1 = ((wc >> 3) & 0xff) as u8;
            let c2 = ((wc >> 11) & 0xff) as u8;
            let c3 = ((wc >> 19) & 0xff) as u8;
            if c1 != 0 {
                s.push(c1 as char);
            }
            if c2 != 0 {
                s.push(c2 as char);
            }
            if c3 != 0 {
                s.push(c3 as char);
            }
            s
        } else {
            // Long string (offset into strs from strs_base)
            let offset = (wc >> 2) as usize;
            self.get_string_at(self.strs_base + offset)
        }
    }

    fn get_string_at(&self, offset: usize) -> String {
        if offset >= self.strings.len() {
            return String::new();
        }

        let end = self.strings[offset..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| offset + p)
            .unwrap_or(self.strings.len());

        // Untokenize the zsh string - convert tokens back to shell syntax
        let raw = &self.strings[offset..end];
        untokenize(raw)
    }

    /// Decode the wordcode into a list of operations
    pub fn decode(&self) -> Vec<DecodedOp> {
        let mut decoder = WordcodeDecoder::new(self.code, self.strings, self.strs_base);
        decoder.decode_program()
    }

    fn decode_program(&mut self) -> Vec<DecodedOp> {
        let mut ops = Vec::new();

        while let Some(wc) = self.peek() {
            let code = wc_code(wc);

            if code == WC_END {
                self.next();
                ops.push(DecodedOp::End);
                break;
            }

            if let Some(op) = self.decode_next_op() {
                ops.push(op);
            } else {
                break;
            }
        }

        ops
    }

    fn decode_next_op(&mut self) -> Option<DecodedOp> {
        let wc = self.next()?;
        let code = wc_code(wc);
        let data = wc_data(wc);

        let op = match code {
            WC_END => DecodedOp::End,
            WC_LIST => self.decode_list(data),
            WC_SUBLIST => self.decode_sublist(data),
            WC_PIPE => self.decode_pipe(data),
            WC_REDIR => self.decode_redir(data),
            WC_ASSIGN => self.decode_assign(data),
            WC_SIMPLE => self.decode_simple(data),
            WC_TYPESET => self.decode_typeset(data),
            WC_SUBSH => self.decode_subsh(data),
            WC_CURSH => self.decode_cursh(data),
            WC_TIMED => self.decode_timed(data),
            WC_FUNCDEF => self.decode_funcdef(data),
            WC_FOR => self.decode_for(data),
            WC_SELECT => self.decode_select(data),
            WC_WHILE => self.decode_while(data),
            WC_REPEAT => self.decode_repeat(data),
            WC_CASE => self.decode_case(data),
            WC_IF => self.decode_if(data),
            WC_COND => self.decode_cond(data),
            WC_ARITH => self.decode_arith(),
            WC_AUTOFN => DecodedOp::AutoFn,
            WC_TRY => self.decode_try(data),
            _ => DecodedOp::Unknown { code, data },
        };

        Some(op)
    }

    fn decode_list(&mut self, data: u32) -> DecodedOp {
        let list_type = data & ((1 << WC_LIST_FREE) - 1);
        let is_end = (list_type & Z_END) != 0;
        let is_simple = (list_type & Z_SIMPLE) != 0;
        let _skip = data >> WC_LIST_FREE;

        let mut body = Vec::new();

        if is_simple {
            // Simple list just has a lineno, then the command
            let lineno = self.next().unwrap_or(0);
            body.push(DecodedOp::LineNo(lineno));
        }

        // Continue decoding the list contents
        if !is_simple {
            while let Some(wc) = self.peek() {
                let c = wc_code(wc);
                if c == WC_END || c == WC_LIST {
                    break;
                }
                if let Some(op) = self.decode_next_op() {
                    body.push(op);
                } else {
                    break;
                }
            }
        }

        DecodedOp::List {
            list_type,
            is_end,
            ops: body,
        }
    }

    fn decode_sublist(&mut self, data: u32) -> DecodedOp {
        let sublist_type = data & 3;
        let flags = data & 0x1c;
        let negated = (flags & WC_SUBLIST_NOT) != 0;
        let is_simple = (flags & WC_SUBLIST_SIMPLE) != 0;
        let _skip = data >> WC_SUBLIST_FREE;

        let mut body = Vec::new();

        if is_simple {
            // Simple sublist
            let lineno = self.next().unwrap_or(0);
            body.push(DecodedOp::LineNo(lineno));
        }

        DecodedOp::Sublist {
            sublist_type,
            negated,
            ops: body,
        }
    }

    fn decode_pipe(&mut self, data: u32) -> DecodedOp {
        let pipe_type = data & 1;
        let lineno = data >> 1;
        let _is_end = pipe_type == WC_PIPE_END;

        DecodedOp::Pipe {
            lineno,
            ops: vec![],
        }
    }

    fn decode_redir(&mut self, data: u32) -> DecodedOp {
        let redir_type = data & 0x1f; // REDIR_TYPE_MASK
        let has_varid = (data & 0x20) != 0; // REDIR_VARID_MASK
        let from_heredoc = (data & 0x40) != 0; // REDIR_FROM_HEREDOC_MASK

        let fd = self.next().unwrap_or(0) as i32;
        let target = self.read_string();

        let varid = if has_varid {
            Some(self.read_string())
        } else {
            None
        };

        if from_heredoc {
            // Skip heredoc data (2 extra words)
            self.next();
            self.next();
        }

        DecodedOp::Redir {
            redir_type,
            fd,
            target,
            varid,
        }
    }

    fn decode_assign(&mut self, data: u32) -> DecodedOp {
        let is_array = (data & 1) != 0;
        let num_elements = (data >> 2) as usize;

        let name = self.read_string();

        if is_array {
            let mut values = Vec::with_capacity(num_elements);
            for _ in 0..num_elements {
                values.push(self.read_string());
            }
            DecodedOp::AssignArray { name, values }
        } else {
            let value = self.read_string();
            DecodedOp::Assign { name, value }
        }
    }

    fn decode_simple(&mut self, data: u32) -> DecodedOp {
        let argc = data as usize;
        let mut args = Vec::with_capacity(argc);
        for _ in 0..argc {
            args.push(self.read_string());
        }
        DecodedOp::Simple { args }
    }

    fn decode_typeset(&mut self, data: u32) -> DecodedOp {
        let argc = data as usize;
        let mut args = Vec::with_capacity(argc);
        for _ in 0..argc {
            args.push(self.read_string());
        }

        // Followed by number of assignments
        let num_assigns = self.next().unwrap_or(0) as usize;
        let mut assigns = Vec::with_capacity(num_assigns);

        for _ in 0..num_assigns {
            if let Some(op) = self.decode_next_op() {
                assigns.push(op);
            }
        }

        DecodedOp::Typeset { args, assigns }
    }

    fn decode_subsh(&mut self, data: u32) -> DecodedOp {
        let skip = data as usize;
        let end_pos = self.pos + skip;

        let mut body = Vec::new();
        while self.pos < end_pos && !self.at_end() {
            if let Some(op) = self.decode_next_op() {
                body.push(op);
            } else {
                break;
            }
        }

        DecodedOp::Subsh { ops: body }
    }

    fn decode_cursh(&mut self, data: u32) -> DecodedOp {
        let skip = data as usize;
        let end_pos = self.pos + skip;

        let mut body = Vec::new();
        while self.pos < end_pos && !self.at_end() {
            if let Some(op) = self.decode_next_op() {
                body.push(op);
            } else {
                break;
            }
        }

        DecodedOp::Cursh { ops: body }
    }

    fn decode_timed(&mut self, data: u32) -> DecodedOp {
        let timed_type = data;
        let has_pipe = timed_type == 1; // WC_TIMED_PIPE

        if has_pipe {
            // Followed by a pipe
            if let Some(op) = self.decode_next_op() {
                return DecodedOp::Timed {
                    cmd: Some(Box::new(op)),
                };
            }
        }

        DecodedOp::Timed { cmd: None }
    }

    fn decode_funcdef(&mut self, data: u32) -> DecodedOp {
        let skip = data as usize;

        let num_names = self.next().unwrap_or(0) as usize;
        let mut names = Vec::with_capacity(num_names);
        for _ in 0..num_names {
            names.push(self.read_string());
        }

        // Read function metadata
        let _strs_offset = self.next();
        let _strs_len = self.next();
        let _npats = self.next();
        let _tracing = self.next();

        // Skip the function body (we'd need a separate decoder for it)
        let _end_pos = self.pos + skip.saturating_sub(num_names + 5);

        let name = names.first().cloned().unwrap_or_default();

        DecodedOp::FuncDef { name, body: vec![] }
    }

    fn decode_for(&mut self, data: u32) -> DecodedOp {
        let for_type = data & 3;
        let _skip = data >> 2;

        match for_type {
            WC_FOR_COND => {
                let init = self.read_string();
                let cond = self.read_string();
                let step = self.read_string();
                DecodedOp::ForCond {
                    init,
                    cond,
                    step,
                    body: vec![],
                }
            }
            WC_FOR_LIST => {
                let var = self.read_string();
                let num_words = self.next().unwrap_or(0) as usize;
                let mut list = Vec::with_capacity(num_words);
                for _ in 0..num_words {
                    list.push(self.read_string());
                }
                DecodedOp::For {
                    var,
                    list,
                    body: vec![],
                }
            }
            _ => {
                // WC_FOR_PPARAM - uses positional params
                let var = self.read_string();
                DecodedOp::For {
                    var,
                    list: vec![],
                    body: vec![],
                }
            }
        }
    }

    fn decode_select(&mut self, data: u32) -> DecodedOp {
        let select_type = data & 1;
        let _skip = data >> 1;

        let var = self.read_string();
        let list = if select_type == 1 {
            // WC_SELECT_LIST
            let num_words = self.next().unwrap_or(0) as usize;
            let mut words = Vec::with_capacity(num_words);
            for _ in 0..num_words {
                words.push(self.read_string());
            }
            words
        } else {
            vec![]
        };

        DecodedOp::Select {
            var,
            list,
            body: vec![],
        }
    }

    fn decode_while(&mut self, data: u32) -> DecodedOp {
        let is_until = (data & 1) != 0;
        let _skip = data >> 1;
        DecodedOp::While {
            cond: vec![],
            body: vec![],
            is_until,
        }
    }

    fn decode_repeat(&mut self, data: u32) -> DecodedOp {
        let _skip = data;
        let count = self.read_string();
        DecodedOp::Repeat {
            count,
            body: vec![],
        }
    }

    fn decode_case(&mut self, data: u32) -> DecodedOp {
        let case_type = data & 7;
        let _skip = data >> WC_CASE_FREE;

        if case_type == WC_CASE_HEAD {
            let word = self.read_string();
            DecodedOp::Case {
                word,
                cases: vec![],
            }
        } else {
            // Individual case patterns
            let pattern = self.read_string();
            let _npats = self.next();
            DecodedOp::CaseItem {
                pattern,
                terminator: case_type,
                body: vec![],
            }
        }
    }

    fn decode_if(&mut self, data: u32) -> DecodedOp {
        let if_type = data & 3;
        let _skip = data >> 2;

        DecodedOp::If {
            if_type,
            conditions: vec![],
            else_body: None,
        }
    }

    fn decode_cond(&mut self, data: u32) -> DecodedOp {
        let cond_type = data & 127;
        let _skip = data >> 7;

        // Decode based on condition type
        let args = match cond_type {
            // COND_NOT = 1
            1 => vec![],
            // COND_AND = 2, COND_OR = 3
            2 | 3 => vec![],
            // Binary operators have 2 args
            _ if cond_type >= 7 => {
                vec![self.read_string(), self.read_string()]
            }
            // Unary operators have 1 arg
            _ => {
                vec![self.read_string()]
            }
        };

        DecodedOp::Cond { cond_type, args }
    }

    fn decode_arith(&mut self) -> DecodedOp {
        let expr = self.read_string();
        DecodedOp::Arith { expr }
    }

    fn decode_try(&mut self, data: u32) -> DecodedOp {
        let _skip = data;
        DecodedOp::Try {
            try_body: vec![],
            always_body: vec![],
        }
    }
}

pub fn dump_zwc_info<P: AsRef<Path>>(path: P) -> io::Result<()> {
    let zwc = ZwcFile::load(&path)?;

    println!("ZWC file: {:?}", path.as_ref());
    println!(
        "  Magic: 0x{:08x} ({})",
        zwc.header.magic,
        if zwc.header.magic == FD_MAGIC {
            "native"
        } else {
            "swapped"
        }
    );
    println!("  Version: zsh-{}", zwc.header.version);
    println!("  Header length: {} words", zwc.header.header_len);
    println!("  Wordcode size: {} words", zwc.wordcode.len());
    println!("  Functions: {}", zwc.functions.len());

    for func in &zwc.functions {
        println!(
            "    {} (offset={}, len={}, npats={})",
            func.name, func.start, func.len, func.npats
        );
    }

    Ok(())
}

pub fn dump_zwc_function<P: AsRef<Path>>(path: P, func_name: &str) -> io::Result<()> {
    let zwc = ZwcFile::load(&path)?;

    let func = zwc.get_function(func_name).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("Function '{}' not found", func_name),
        )
    })?;

    println!("Function: {}", func.name);
    println!("  Offset: {} words", func.start);
    println!("  Length: {} words", func.len);
    println!("  Patterns: {}", func.npats);
    println!("  Strings offset: {}", func.strs_offset);

    // Show raw wordcode
    let header_words = zwc.header.header_len as usize;
    let start_idx = (func.start as usize).saturating_sub(header_words);
    let end_idx = start_idx + func.len as usize;

    if start_idx < zwc.wordcode.len() {
        println!("\n  Wordcode:");
        let end = end_idx.min(zwc.wordcode.len());
        for (i, &wc) in zwc.wordcode[start_idx..end].iter().enumerate().take(50) {
            let code = wc_code(wc);
            let data = wc_data(wc);
            let code_name = match code {
                WC_END => "END",
                WC_LIST => "LIST",
                WC_SUBLIST => "SUBLIST",
                WC_PIPE => "PIPE",
                WC_REDIR => "REDIR",
                WC_ASSIGN => "ASSIGN",
                WC_SIMPLE => "SIMPLE",
                WC_TYPESET => "TYPESET",
                WC_SUBSH => "SUBSH",
                WC_CURSH => "CURSH",
                WC_TIMED => "TIMED",
                WC_FUNCDEF => "FUNCDEF",
                WC_FOR => "FOR",
                WC_SELECT => "SELECT",
                WC_WHILE => "WHILE",
                WC_REPEAT => "REPEAT",
                WC_CASE => "CASE",
                WC_IF => "IF",
                WC_COND => "COND",
                WC_ARITH => "ARITH",
                WC_AUTOFN => "AUTOFN",
                WC_TRY => "TRY",
                _ => "???",
            };
            println!("    [{:3}] 0x{:08x} = {} (data={})", i, wc, code_name, data);
        }
        if end - start_idx > 50 {
            println!("    ... ({} more words)", end - start_idx - 50);
        }
    }

    // Try to decode
    if let Some(decoded) = zwc.decode_function(func) {
        println!("\n  Decoded ops:");
        for (i, op) in decoded.body.iter().enumerate().take(20) {
            println!("    [{:2}] {:?}", i, op);
        }
        if decoded.body.len() > 20 {
            println!("    ... ({} more ops)", decoded.body.len() - 20);
        }
    }

    Ok(())
}

/// Convert decoded ZWC ops to our shell AST for execution
impl DecodedOp {
    pub fn to_shell_command(&self) -> Option<ShellCommand> {
        match self {
            DecodedOp::Simple { args } => {
                if args.is_empty() {
                    return None;
                }
                Some(ShellCommand::Simple(SimpleCommand {
                    assignments: vec![],
                    words: args.iter().map(|s| ShellWord::Literal(s.clone())).collect(),
                    redirects: vec![],
                }))
            }

            DecodedOp::Assign { name, value } => Some(ShellCommand::Simple(SimpleCommand {
                assignments: vec![(name.clone(), ShellWord::Literal(value.clone()), false)],
                words: vec![],
                redirects: vec![],
            })),

            DecodedOp::AssignArray { name, values } => {
                let array_word = ShellWord::Concat(
                    values
                        .iter()
                        .map(|s| ShellWord::Literal(s.clone()))
                        .collect(),
                );
                Some(ShellCommand::Simple(SimpleCommand {
                    assignments: vec![(name.clone(), array_word, false)],
                    words: vec![],
                    redirects: vec![],
                }))
            }

            DecodedOp::List { ops, .. } => {
                let commands: Vec<(ShellCommand, ListOp)> = ops
                    .iter()
                    .filter_map(|op| op.to_shell_command())
                    .map(|cmd| (cmd, ListOp::Semi))
                    .collect();

                if commands.is_empty() {
                    None
                } else if commands.len() == 1 {
                    Some(commands.into_iter().next().unwrap().0)
                } else {
                    Some(ShellCommand::List(commands))
                }
            }

            DecodedOp::Sublist { ops, negated, .. } => {
                let commands: Vec<ShellCommand> =
                    ops.iter().filter_map(|op| op.to_shell_command()).collect();

                if commands.is_empty() {
                    None
                } else {
                    Some(ShellCommand::Pipeline(commands, *negated))
                }
            }

            DecodedOp::Pipe { ops, .. } => {
                let commands: Vec<ShellCommand> =
                    ops.iter().filter_map(|op| op.to_shell_command()).collect();

                if commands.is_empty() {
                    None
                } else if commands.len() == 1 {
                    Some(commands.into_iter().next().unwrap())
                } else {
                    Some(ShellCommand::Pipeline(commands, false))
                }
            }

            DecodedOp::Typeset { args, assigns } => {
                // Typeset is like a simple command with the typeset builtin
                let mut words: Vec<ShellWord> =
                    args.iter().map(|s| ShellWord::Literal(s.clone())).collect();

                // Add any assignments as words
                for assign in assigns {
                    if let DecodedOp::Assign { name, value } = assign {
                        words.push(ShellWord::Literal(format!("{}={}", name, value)));
                    }
                }

                Some(ShellCommand::Simple(SimpleCommand {
                    assignments: vec![],
                    words,
                    redirects: vec![],
                }))
            }

            DecodedOp::Subsh { ops } => {
                let commands: Vec<ShellCommand> =
                    ops.iter().filter_map(|op| op.to_shell_command()).collect();
                Some(ShellCommand::Compound(CompoundCommand::Subshell(commands)))
            }

            DecodedOp::Cursh { ops } => {
                let commands: Vec<ShellCommand> =
                    ops.iter().filter_map(|op| op.to_shell_command()).collect();
                Some(ShellCommand::Compound(CompoundCommand::BraceGroup(
                    commands,
                )))
            }

            DecodedOp::For { var, list, body } => {
                let words = if list.is_empty() {
                    None
                } else {
                    Some(list.iter().map(|s| ShellWord::Literal(s.clone())).collect())
                };
                let body_cmds: Vec<ShellCommand> =
                    body.iter().filter_map(|op| op.to_shell_command()).collect();
                Some(ShellCommand::Compound(CompoundCommand::For {
                    var: var.clone(),
                    words,
                    body: body_cmds,
                }))
            }

            DecodedOp::ForCond {
                init,
                cond,
                step,
                body,
            } => {
                let body_cmds: Vec<ShellCommand> =
                    body.iter().filter_map(|op| op.to_shell_command()).collect();
                Some(ShellCommand::Compound(CompoundCommand::ForArith {
                    init: init.clone(),
                    cond: cond.clone(),
                    step: step.clone(),
                    body: body_cmds,
                }))
            }

            DecodedOp::While {
                cond,
                body,
                is_until,
            } => {
                let cond_cmds: Vec<ShellCommand> =
                    cond.iter().filter_map(|op| op.to_shell_command()).collect();
                let body_cmds: Vec<ShellCommand> =
                    body.iter().filter_map(|op| op.to_shell_command()).collect();

                if *is_until {
                    Some(ShellCommand::Compound(CompoundCommand::Until {
                        condition: cond_cmds,
                        body: body_cmds,
                    }))
                } else {
                    Some(ShellCommand::Compound(CompoundCommand::While {
                        condition: cond_cmds,
                        body: body_cmds,
                    }))
                }
            }

            DecodedOp::FuncDef { name, body } => {
                let body_cmds: Vec<ShellCommand> =
                    body.iter().filter_map(|op| op.to_shell_command()).collect();

                let func_body = if body_cmds.is_empty() {
                    // Empty function body - create a no-op
                    ShellCommand::Simple(SimpleCommand {
                        assignments: vec![],
                        words: vec![ShellWord::Literal(":".to_string())],
                        redirects: vec![],
                    })
                } else if body_cmds.len() == 1 {
                    body_cmds.into_iter().next().unwrap()
                } else {
                    ShellCommand::List(body_cmds.into_iter().map(|c| (c, ListOp::Semi)).collect())
                };

                Some(ShellCommand::FunctionDef(name.clone(), Box::new(func_body)))
            }

            DecodedOp::Arith { expr } => {
                Some(ShellCommand::Compound(CompoundCommand::Arith(expr.clone())))
            }

            // Ops that don't directly translate
            DecodedOp::End | DecodedOp::LineNo(_) | DecodedOp::AutoFn => None,

            DecodedOp::Redir { .. } => {
                // Redirections are attached to commands, not standalone
                None
            }

            DecodedOp::If { .. }
            | DecodedOp::Case { .. }
            | DecodedOp::CaseItem { .. }
            | DecodedOp::Select { .. }
            | DecodedOp::Cond { .. }
            | DecodedOp::Repeat { .. }
            | DecodedOp::Try { .. }
            | DecodedOp::Timed { .. }
            | DecodedOp::Unknown { .. } => {
                // TODO: Implement these
                None
            }
        }
    }
}

/// Helper to convert redir type to our RedirectOp
#[allow(dead_code)]
fn redir_type_to_op(redir_type: u32) -> Option<RedirectOp> {
    // Zsh redirect types from zsh.h
    const REDIR_WRITE: u32 = 0;
    const REDIR_WRITENOW: u32 = 1;
    const REDIR_APP: u32 = 2;
    const REDIR_APPNOW: u32 = 3;
    const REDIR_ERRWRITE: u32 = 4;
    const REDIR_ERRWRITENOW: u32 = 5;
    const REDIR_ERRAPP: u32 = 6;
    const REDIR_ERRAPPNOW: u32 = 7;
    const REDIR_READWRITE: u32 = 8;
    const REDIR_READ: u32 = 9;
    const REDIR_HEREDOC: u32 = 10;
    const REDIR_HEREDOCDASH: u32 = 11;
    const REDIR_HERESTR: u32 = 12;
    const REDIR_MERGEIN: u32 = 13;
    const REDIR_MERGEOUT: u32 = 14;
    const REDIR_CLOSE: u32 = 15;
    const REDIR_INPIPE: u32 = 16;
    const REDIR_OUTPIPE: u32 = 17;

    match redir_type {
        REDIR_WRITE | REDIR_WRITENOW => Some(RedirectOp::Write),
        REDIR_APP | REDIR_APPNOW => Some(RedirectOp::Append),
        REDIR_ERRWRITE | REDIR_ERRWRITENOW => Some(RedirectOp::WriteBoth),
        REDIR_ERRAPP | REDIR_ERRAPPNOW => Some(RedirectOp::AppendBoth),
        REDIR_READWRITE => Some(RedirectOp::ReadWrite),
        REDIR_READ => Some(RedirectOp::Read),
        REDIR_HEREDOC | REDIR_HEREDOCDASH => Some(RedirectOp::HereDoc),
        REDIR_HERESTR => Some(RedirectOp::HereString),
        REDIR_MERGEIN => Some(RedirectOp::DupRead),
        REDIR_MERGEOUT => Some(RedirectOp::DupWrite),
        REDIR_CLOSE | REDIR_INPIPE | REDIR_OUTPIPE => None, // Not directly supported
        _ => None,
    }
}

impl DecodedFunction {
    /// Convert the decoded function to a shell function definition
    pub fn to_shell_function(&self) -> Option<ShellCommand> {
        let body_cmds: Vec<ShellCommand> = self
            .body
            .iter()
            .filter_map(|op| op.to_shell_command())
            .collect();

        let func_body = if body_cmds.is_empty() {
            ShellCommand::Simple(SimpleCommand {
                assignments: vec![],
                words: vec![ShellWord::Literal(":".to_string())],
                redirects: vec![],
            })
        } else if body_cmds.len() == 1 {
            body_cmds.into_iter().next().unwrap()
        } else {
            ShellCommand::List(body_cmds.into_iter().map(|c| (c, ListOp::Semi)).collect())
        };

        // Extract just the function name without the path prefix
        let name = self
            .name
            .rsplit('/')
            .next()
            .unwrap_or(&self.name)
            .to_string();

        Some(ShellCommand::FunctionDef(name, Box::new(func_body)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wc_code() {
        assert_eq!(wc_code(WC_LIST), WC_LIST);
        assert_eq!(wc_code(WC_SIMPLE | (5 << WC_CODEBITS)), WC_SIMPLE);
    }

    #[test]
    fn test_wc_data() {
        let wc = WC_SIMPLE | (42 << WC_CODEBITS);
        assert_eq!(wc_data(wc), 42);
    }

    #[test]
    fn test_load_src_zwc() {
        let path = "/Users/wizard/.zinit/plugins/MenkeTechnologies---zsh-more-completions/src.zwc";
        if !std::path::Path::new(path).exists() {
            eprintln!("Skipping test - {} not found", path);
            return;
        }

        let zwc = ZwcFile::load(path).expect("Failed to load src.zwc");
        println!("Loaded {} functions from src.zwc", zwc.function_count());

        // Should have thousands of completion functions
        assert!(
            zwc.function_count() > 1000,
            "Expected > 1000 functions, got {}",
            zwc.function_count()
        );

        // Check some known functions exist
        let funcs = zwc.list_functions();
        println!("First 10 functions: {:?}", &funcs[..10.min(funcs.len())]);

        // Try to decode _ls
        if let Some(func) = zwc.get_function("_ls") {
            println!("Found _ls function");
            if let Some(decoded) = zwc.decode_function(func) {
                println!("Decoded _ls: {} ops", decoded.body.len());
            }
        }
    }

    #[test]
    fn test_load_zshrc_zwc() {
        let home = std::env::var("HOME").unwrap_or_default();
        let path = format!("{}/.zshrc.zwc", home);
        if !std::path::Path::new(&path).exists() {
            eprintln!("Skipping test - {} not found", path);
            return;
        }

        let zwc = ZwcFile::load(&path).expect("Failed to load .zshrc.zwc");
        println!("Loaded {} functions from .zshrc.zwc", zwc.function_count());

        for name in zwc.list_functions() {
            println!("  Function: {}", name);
            if let Some(func) = zwc.get_function(name) {
                if let Some(decoded) = zwc.decode_function(func) {
                    println!("    Decoded: {} ops", decoded.body.len());
                    for (i, op) in decoded.body.iter().take(3).enumerate() {
                        if let Some(cmd) = op.to_shell_command() {
                            println!("      [{}] -> ShellCommand OK", i);
                        } else {
                            println!("      [{}] {:?}", i, op);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_load_zshenv_zwc() {
        let home = std::env::var("HOME").unwrap_or_default();
        let path = format!("{}/.zshenv.zwc", home);
        if !std::path::Path::new(&path).exists() {
            eprintln!("Skipping test - {} not found", path);
            return;
        }

        let zwc = ZwcFile::load(&path).expect("Failed to load .zshenv.zwc");
        println!("Loaded {} functions from .zshenv.zwc", zwc.function_count());

        for name in zwc.list_functions() {
            println!("  Function: {}", name);
            if let Some(func) = zwc.get_function(name) {
                if let Some(decoded) = zwc.decode_function(func) {
                    println!("    Decoded: {} ops", decoded.body.len());
                }
            }
        }
    }
}
