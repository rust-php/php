//! Resources and streams (files, memory, standard streams, directory handles).
use std::{
    cell::{Cell, RefCell},
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    rc::Rc,
};

thread_local! {
    static NEXT_ID: Cell<i64> = const { Cell::new(5) };
    static STD: RefCell<Vec<Rc<Resource>>> = const { RefCell::new(Vec::new()) };
}

pub enum Stream {
    File { file: File, path: String, mode: String, rbuf: Vec<u8>, rpos: usize, eof: bool },
    Memory { data: Vec<u8>, pos: usize, eof: bool },
    Stdin,
    Stdout,
    Stderr,
    /// php://output: goes through the output buffers.
    Output,
    Dir { entries: Vec<String>, pos: usize },
}

pub struct Resource {
    pub id: i64,
    pub kind: &'static str,
    pub stream: RefCell<Option<Stream>>,
}

impl std::fmt::Debug for Resource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "resource#{}", self.id)
    }
}

impl Resource {
    pub fn new(stream: Stream, kind: &'static str) -> Rc<Resource> {
        let id = NEXT_ID.with(|n| {
            let v = n.get();
            n.set(v + 1);
            v
        });
        Rc::new(Resource { id, kind, stream: RefCell::new(Some(stream)) })
    }

    pub fn is_closed(&self) -> bool {
        self.stream.borrow().is_none()
    }
}

/// STDIN (1), STDOUT (2) and STDERR (3).
pub fn std_resource(n: usize) -> Rc<Resource> {
    STD.with(|s| {
        let mut s = s.borrow_mut();
        if s.is_empty() {
            for (i, st) in [Stream::Stdin, Stream::Stdout, Stream::Stderr].into_iter().enumerate() {
                s.push(Rc::new(Resource { id: i as i64 + 1, kind: "stream", stream: RefCell::new(Some(st)) }));
            }
        }
        s[n].clone()
    })
}

impl Stream {
    pub fn open(path: &str, mode: &str) -> Option<Stream> {
        let lower = path.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("php://") {
            return Some(match rest.split('/').next().unwrap_or("") {
                "memory" | "temp" => Stream::Memory { data: vec![], pos: 0, eof: false },
                "stdin" => Stream::Stdin,
                "stdout" => Stream::Stdout,
                "stderr" => Stream::Stderr,
                "output" => Stream::Output,
                _ => return None,
            });
        }
        let path = path.strip_prefix("file://").unwrap_or(path);
        let base = mode.replace(['b', 't'], "");
        let plus = base.contains('+');
        let mut o = std::fs::OpenOptions::new();
        match base.chars().next()? {
            'r' => {
                o.read(true).write(plus);
            }
            'w' => {
                o.write(true).read(plus).create(true).truncate(true);
            }
            'a' => {
                o.append(true).read(plus).create(true);
            }
            'x' => {
                o.write(true).read(plus).create_new(true);
            }
            'c' => {
                o.write(true).read(plus).create(true);
            }
            _ => return None,
        }
        let file = o.open(path).ok()?;
        if file.metadata().ok()?.is_dir() && !base.starts_with('r') {
            return None;
        }
        Some(Stream::File { file, path: path.to_string(), mode: mode.to_string(), rbuf: vec![], rpos: 0, eof: false })
    }

    fn fill(&mut self) -> bool {
        match self {
            Stream::File { file, rbuf, rpos, eof, .. } => {
                let mut buf = vec![0u8; 8192];
                let n = file.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    *eof = true;
                    return false;
                }
                *rbuf = buf[..n].to_vec();
                *rpos = 0;
                true
            }
            _ => false,
        }
    }

    pub fn read(&mut self, n: usize) -> Vec<u8> {
        match self {
            Stream::Memory { data, pos, eof } => {
                let end = (*pos + n).min(data.len());
                let out = data[(*pos).min(end)..end].to_vec();
                *pos = end;
                if out.len() < n {
                    *eof = true;
                }
                out
            }
            Stream::Stdin => {
                let mut buf = vec![0u8; n];
                let got = std::io::stdin().read(&mut buf).unwrap_or(0);
                buf.truncate(got);
                buf
            }
            Stream::File { .. } => {
                let mut out = Vec::new();
                while out.len() < n {
                    let (avail, have) = match self {
                        Stream::File { rbuf, rpos, .. } => (rbuf.len() - *rpos, true),
                        _ => (0, false),
                    };
                    if have && avail > 0 {
                        if let Stream::File { rbuf, rpos, .. } = self {
                            let take = avail.min(n - out.len());
                            out.extend_from_slice(&rbuf[*rpos..*rpos + take]);
                            *rpos += take;
                        }
                    } else if !self.fill() {
                        break;
                    }
                }
                out
            }
            _ => vec![],
        }
    }

    /// Read up to and including `\n` (at most `max` bytes).
    pub fn read_line(&mut self, max: usize) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            if out.len() >= max {
                break;
            }
            let b = self.read(1);
            match b.first() {
                Some(&c) => {
                    out.push(c);
                    if c == b'\n' {
                        break;
                    }
                }
                None => break,
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }

    pub fn write(&mut self, bytes: &[u8]) -> Option<usize> {
        match self {
            Stream::File { file, mode, rbuf, rpos, .. } => {
                if !mode.contains(['w', 'a', 'x', 'c', '+']) {
                    return None;
                }
                let unread = rbuf.len() - *rpos;
                if unread > 0 {
                    let _ = file.seek(SeekFrom::Current(-(unread as i64)));
                }
                rbuf.clear();
                *rpos = 0;
                file.write_all(bytes).ok().map(|_| bytes.len())
            }
            Stream::Memory { data, pos, .. } => {
                let end = *pos + bytes.len();
                if data.len() < end {
                    data.resize(end, 0);
                }
                data[*pos..end].copy_from_slice(bytes);
                *pos = end;
                Some(bytes.len())
            }
            Stream::Stderr => {
                let _ = std::io::stderr().write_all(bytes);
                Some(bytes.len())
            }
            _ => None,
        }
    }

    pub fn tell(&mut self) -> i64 {
        match self {
            Stream::File { file, rbuf, rpos, .. } => file.stream_position().map(|p| p as i64 - (rbuf.len() - *rpos) as i64).unwrap_or(0),
            Stream::Memory { pos, .. } => *pos as i64,
            _ => 0,
        }
    }

    pub fn seek(&mut self, off: i64, whence: i64) -> bool {
        match self {
            Stream::File { file, rbuf, rpos, eof, .. } => {
                let target = match whence {
                    1 => SeekFrom::Current(off - (rbuf.len() - *rpos) as i64),
                    2 => SeekFrom::End(off),
                    _ => SeekFrom::Start(off.max(0) as u64),
                };
                rbuf.clear();
                *rpos = 0;
                *eof = false;
                file.seek(target).is_ok()
            }
            Stream::Memory { data, pos, eof } => {
                let base = match whence {
                    1 => *pos as i64,
                    2 => data.len() as i64,
                    _ => 0,
                };
                if base + off < 0 {
                    return false;
                }
                *pos = (base + off) as usize;
                *eof = false;
                true
            }
            _ => false,
        }
    }

    pub fn eof(&self) -> bool {
        match self {
            Stream::File { eof, rbuf, rpos, .. } => *eof && *rpos >= rbuf.len(),
            Stream::Memory { eof, .. } => *eof,
            Stream::Stdin => false,
            _ => false,
        }
    }

    pub fn truncate(&mut self, size: usize) -> bool {
        match self {
            Stream::File { file, .. } => file.set_len(size as u64).is_ok(),
            Stream::Memory { data, .. } => {
                data.resize(size, 0);
                true
            }
            _ => false,
        }
    }

    pub fn size(&self) -> Option<u64> {
        match self {
            Stream::File { file, .. } => file.metadata().ok().map(|m| m.len()),
            Stream::Memory { data, .. } => Some(data.len() as u64),
            _ => None,
        }
    }
}
