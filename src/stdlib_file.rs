//! File system, streams and path helpers.
use crate::builtins::{arg, arr, s, Builtin, VmBuiltin};
use crate::stream::{Resource, Stream};
use crate::value::*;
use crate::vm::{Flow, Vm};
use std::{collections::HashMap, io::Write, rc::Rc};

type R<T> = Result<T, Flow>;

fn res(a: &[Value], i: usize) -> Option<Rc<Resource>> {
    match a.get(i) {
        Some(Value::Resource(r)) => Some(r.clone()),
        _ => None,
    }
}

fn lossy(b: &[u8]) -> Value {
    Value::Str(String::from_utf8_lossy(b).to_string().into())
}

fn ok(b: bool) -> R<Value> {
    Ok(Value::Bool(b))
}

fn bad_resource<T>(vm: &mut Vm<'_>, f: &str, a: &[Value]) -> R<T> {
    vm.throw_error("TypeError", &format!("{f}(): Argument #1 ($stream) must be of type resource, {} given", arg(a, 0).type_name()))
}

/// Run `f` on an open stream, or fail the way PHP does for closed/invalid resources.
fn with_stream<T>(vm: &mut Vm<'_>, name: &str, a: &[Value], f: impl FnOnce(&mut Stream, &mut Vm<'_>) -> T) -> R<T> {
    let Some(r) = res(a, 0) else { return bad_resource(vm, name, a) };
    let taken = r.stream.borrow_mut().take();
    match taken {
        Some(mut st) => {
            let out = f(&mut st, vm);
            *r.stream.borrow_mut() = Some(st);
            Ok(out)
        }
        None => vm.throw_error("TypeError", &format!("{name}(): supplied resource is not a valid stream resource")),
    }
}

pub fn fnmatch(pat: &[char], text: &[char]) -> bool {
    match pat.first() {
        None => text.is_empty(),
        Some('*') => (0..=text.len()).any(|i| fnmatch(&pat[1..], &text[i..])),
        Some('?') => !text.is_empty() && fnmatch(&pat[1..], &text[1..]),
        Some('[') => {
            let Some(end) = pat.iter().skip(1).position(|c| *c == ']').map(|p| p + 1) else {
                return text.first() == Some(&'[') && fnmatch(&pat[1..], &text[1..]);
            };
            let Some(&c) = text.first() else { return false };
            let set = &pat[1..end];
            let (neg, set) = if matches!(set.first(), Some('!' | '^')) { (true, &set[1..]) } else { (false, set) };
            let mut hit = false;
            let mut i = 0;
            while i < set.len() {
                if i + 2 < set.len() && set[i + 1] == '-' {
                    hit |= set[i] <= c && c <= set[i + 2];
                    i += 3;
                } else {
                    hit |= set[i] == c;
                    i += 1;
                }
            }
            hit != neg && fnmatch(&pat[end + 1..], &text[1..])
        }
        Some('\\') if pat.len() > 1 => text.first() == Some(&pat[1]) && fnmatch(&pat[2..], &text[1..]),
        Some(c) => text.first() == Some(c) && fnmatch(&pat[1..], &text[1..]),
    }
}

fn glob(pattern: &str) -> Vec<String> {
    let (dir, file_pat) = match pattern.rfind('/') {
        Some(i) => (&pattern[..i.max(1)], &pattern[i + 1..]),
        None => (".", pattern),
    };
    let dir_wild = dir.contains(['*', '?', '[']);
    let dirs: Vec<String> = if dir_wild { glob(dir) } else { vec![dir.to_string()] };
    let pat: Vec<char> = file_pat.chars().collect();
    let mut out = Vec::new();
    for d in dirs {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') && !file_pat.starts_with('.') {
                    continue;
                }
                if fnmatch(&pat, &name.chars().collect::<Vec<_>>()) {
                    out.push(if pattern.contains('/') { format!("{}/{}", if d == "/" { "" } else { &d }, name) } else { name });
                }
            }
        }
    }
    out.sort();
    out
}

fn basename(path: &str, suffix: &str) -> String {
    let t = path.trim_end_matches('/');
    let base = t.rsplit('/').next().unwrap_or("");
    if !suffix.is_empty() && base.ends_with(suffix) && base != suffix {
        base[..base.len() - suffix.len()].to_string()
    } else {
        base.to_string()
    }
}

fn dirname(path: &str) -> String {
    let t = path.trim_end_matches('/');
    if t.is_empty() {
        return if path.starts_with('/') { "/".into() } else { ".".into() };
    }
    match t.rfind('/') {
        None => ".".into(),
        Some(0) => "/".into(),
        Some(i) => t[..i].trim_end_matches('/').to_string(),
    }
}

fn csv_parse(line: &str, delim: char, encl: char) -> Vec<Value> {
    let chars: Vec<char> = line.trim_end_matches(['\n', '\r']).chars().collect();
    if chars.is_empty() {
        return vec![Value::Null];
    }
    let mut out = Vec::new();
    let (mut i, mut cur, mut quoted) = (0, String::new(), false);
    while i < chars.len() {
        let c = chars[i];
        if quoted {
            if c == encl && chars.get(i + 1) == Some(&encl) {
                cur.push(encl);
                i += 1;
            } else if c == encl {
                quoted = false;
            } else {
                cur.push(c);
            }
        } else if c == encl {
            quoted = true;
        } else if c == delim {
            out.push(Value::str(&std::mem::take(&mut cur)));
        } else {
            cur.push(c);
        }
        i += 1;
    }
    out.push(Value::str(&cur));
    out
}

fn csv_line(fields: &[Value], delim: &str, encl: &str) -> String {
    fields
        .iter()
        .map(|f| {
            let t = f.to_str();
            if t.contains(delim) || t.contains(encl) || t.contains(['\n', '\r', '\t', ' ']) {
                format!("{encl}{}{encl}", t.replace(encl, &format!("{encl}{encl}")))
            } else {
                t.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(delim)
        + "\n"
}

fn stat_array(m: &std::fs::Metadata) -> Value {
    use std::os::unix::fs::MetadataExt;
    let mut a = PhpArray::new();
    for (k, v) in [
        ("dev", m.dev() as i64),
        ("ino", m.ino() as i64),
        ("mode", m.mode() as i64),
        ("nlink", m.nlink() as i64),
        ("uid", m.uid() as i64),
        ("gid", m.gid() as i64),
        ("rdev", m.rdev() as i64),
        ("size", m.size() as i64),
        ("atime", m.atime()),
        ("mtime", m.mtime()),
        ("ctime", m.ctime()),
        ("blksize", m.blksize() as i64),
        ("blocks", m.blocks() as i64),
    ] {
        a.set(Key::from_str(k), Value::Int(v));
    }
    Value::Array(Rc::new(a))
}

fn parse_url(url: &str) -> PhpArray {
    let mut out = PhpArray::new();
    let mut rest = url;
    let put = |out: &mut PhpArray, k: &str, v: &str| out.set(Key::from_str(k), Value::str(v));
    if let Some(i) = rest.find('#') {
        put(&mut out, "fragment", &rest[i + 1..]);
        rest = &rest[..i];
    }
    let mut query = None;
    if let Some(i) = rest.find('?') {
        query = Some(rest[i + 1..].to_string());
        rest = &rest[..i];
    }
    let mut result = PhpArray::new();
    if let Some(i) = rest.find("://") {
        put(&mut result, "scheme", &rest[..i]);
        rest = &rest[i + 3..];
        let (authority, path) = match rest.find('/') {
            Some(j) => (&rest[..j], &rest[j..]),
            None => (rest, ""),
        };
        let (userinfo, hostport) = match authority.rfind('@') {
            Some(j) => (Some(&authority[..j]), &authority[j + 1..]),
            None => (None, authority),
        };
        match hostport.rsplit_once(':') {
            Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => {
                put(&mut result, "host", h);
                result.set(Key::from_str("port"), Value::Int(p.parse().unwrap_or(0)));
            }
            _ => put(&mut result, "host", hostport),
        }
        if let Some(u) = userinfo {
            match u.split_once(':') {
                Some((user, pass)) => {
                    put(&mut result, "user", user);
                    put(&mut result, "pass", pass);
                }
                None => put(&mut result, "user", u),
            }
        }
        if !path.is_empty() {
            put(&mut result, "path", path);
        }
    } else if !rest.is_empty() {
        put(&mut result, "path", rest);
    }
    if let Some(q) = query {
        put(&mut result, "query", &q);
    }
    for (k, v) in out.iter() {
        result.set(k.clone(), v.clone());
    }
    result
}

pub fn register_simple(m: &mut HashMap<&'static str, Builtin>) {
    let list: &[(&str, Builtin)] = &[
        ("basename", |_, a| Ok(s(&basename(&arg(&a, 0).to_str(), &a.get(1).map(|v| v.to_str().to_string()).unwrap_or_default())))),
        ("dirname", |_, a| {
            let mut p = arg(&a, 0).to_str().to_string();
            for _ in 0..a.get(1).map(Value::to_int).unwrap_or(1).max(1) {
                p = dirname(&p);
            }
            Ok(s(&p))
        }),
        ("pathinfo", |_, a| {
            let path = arg(&a, 0).to_str();
            let base = basename(&path, "");
            let mut out = PhpArray::new();
            if path.contains('/') {
                out.set(Key::from_str("dirname"), s(&dirname(&path)));
            } else {
                out.set(Key::from_str("dirname"), s("."));
            }
            out.set(Key::from_str("basename"), s(&base));
            if let Some(i) = base.rfind('.') {
                out.set(Key::from_str("extension"), s(&base[i + 1..]));
                out.set(Key::from_str("filename"), s(&base[..i]));
            } else {
                out.set(Key::from_str("filename"), s(&base));
            }
            Ok(match a.get(1).map(Value::to_int) {
                Some(1) => out.get(&Key::from_str("dirname")).cloned().unwrap_or(s("")),
                Some(2) => out.get(&Key::from_str("basename")).cloned().unwrap_or(s("")),
                Some(4) => out.get(&Key::from_str("extension")).cloned().unwrap_or(s("")),
                Some(8) => out.get(&Key::from_str("filename")).cloned().unwrap_or(s("")),
                _ => Value::Array(Rc::new(out)),
            })
        }),
        ("fnmatch", |_, a| Ok(Value::Bool(fnmatch(&arg(&a, 0).to_str().chars().collect::<Vec<_>>(), &arg(&a, 1).to_str().chars().collect::<Vec<_>>())))),
        ("glob", |_, a| Ok(arr(glob(&arg(&a, 0).to_str()).iter().map(|p| s(p)).collect()))),
        ("realpath", |_, a| Ok(std::fs::canonicalize(&*arg(&a, 0).to_str()).map(|p| s(&p.display().to_string())).unwrap_or(Value::Bool(false)))),
        ("file_exists", |_, a| Ok(Value::Bool(std::path::Path::new(&*arg(&a, 0).to_str()).exists()))),
        ("is_file", |_, a| Ok(Value::Bool(std::path::Path::new(&*arg(&a, 0).to_str()).is_file()))),
        ("is_dir", |_, a| Ok(Value::Bool(std::path::Path::new(&*arg(&a, 0).to_str()).is_dir()))),
        ("is_link", |_, a| Ok(Value::Bool(std::fs::symlink_metadata(&*arg(&a, 0).to_str()).is_ok_and(|m| m.file_type().is_symlink())))),
        ("is_readable", |_, a| Ok(Value::Bool(std::fs::File::open(&*arg(&a, 0).to_str()).is_ok() || std::path::Path::new(&*arg(&a, 0).to_str()).is_dir()))),
        ("is_writable", |_, a| Ok(Value::Bool(std::fs::metadata(&*arg(&a, 0).to_str()).is_ok_and(|m| !m.permissions().readonly())))),
        ("is_writeable", |_, a| Ok(Value::Bool(std::fs::metadata(&*arg(&a, 0).to_str()).is_ok_and(|m| !m.permissions().readonly())))),
        ("is_executable", |_, a| {
            use std::os::unix::fs::PermissionsExt;
            Ok(Value::Bool(std::fs::metadata(&*arg(&a, 0).to_str()).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)))
        }),
        ("filesize", |_, a| Ok(std::fs::metadata(&*arg(&a, 0).to_str()).map(|m| Value::Int(m.len() as i64)).unwrap_or(Value::Bool(false)))),
        ("filemtime", |_, a| {
            use std::os::unix::fs::MetadataExt;
            Ok(std::fs::metadata(&*arg(&a, 0).to_str()).map(|m| Value::Int(m.mtime())).unwrap_or(Value::Bool(false)))
        }),
        ("fileatime", |_, a| {
            use std::os::unix::fs::MetadataExt;
            Ok(std::fs::metadata(&*arg(&a, 0).to_str()).map(|m| Value::Int(m.atime())).unwrap_or(Value::Bool(false)))
        }),
        ("filectime", |_, a| {
            use std::os::unix::fs::MetadataExt;
            Ok(std::fs::metadata(&*arg(&a, 0).to_str()).map(|m| Value::Int(m.ctime())).unwrap_or(Value::Bool(false)))
        }),
        ("fileperms", |_, a| {
            use std::os::unix::fs::MetadataExt;
            Ok(std::fs::metadata(&*arg(&a, 0).to_str()).map(|m| Value::Int(m.mode() as i64)).unwrap_or(Value::Bool(false)))
        }),
        ("stat", |_, a| Ok(std::fs::metadata(&*arg(&a, 0).to_str()).map(|m| stat_array(&m)).unwrap_or(Value::Bool(false)))),
        ("clearstatcache", |_, _| Ok(Value::Null)),
        ("unlink", |_, a| Ok(Value::Bool(std::fs::remove_file(&*arg(&a, 0).to_str()).is_ok()))),
        ("rmdir", |_, a| Ok(Value::Bool(std::fs::remove_dir(&*arg(&a, 0).to_str()).is_ok()))),
        ("mkdir", |_, a| {
            let p = arg(&a, 0).to_str();
            let r = if arg(&a, 2).to_bool() { std::fs::create_dir_all(&*p) } else { std::fs::create_dir(&*p) };
            Ok(Value::Bool(r.is_ok()))
        }),
        ("rename", |_, a| Ok(Value::Bool(std::fs::rename(&*arg(&a, 0).to_str(), &*arg(&a, 1).to_str()).is_ok()))),
        ("copy", |_, a| Ok(Value::Bool(std::fs::copy(&*arg(&a, 0).to_str(), &*arg(&a, 1).to_str()).is_ok()))),
        ("touch", |_, a| {
            let p = arg(&a, 0).to_str();
            let r = std::fs::OpenOptions::new().create(true).append(true).open(&*p);
            Ok(Value::Bool(r.is_ok()))
        }),
        ("chmod", |_, a| {
            use std::os::unix::fs::PermissionsExt;
            let r = std::fs::set_permissions(&*arg(&a, 0).to_str(), std::fs::Permissions::from_mode(arg(&a, 1).to_int() as u32));
            Ok(Value::Bool(r.is_ok()))
        }),
        ("sys_get_temp_dir", |_, _| Ok(s("/tmp"))),
        ("getcwd", |_, _| Ok(std::env::current_dir().map(|p| s(&p.display().to_string())).unwrap_or(Value::Bool(false)))),
        ("chdir", |_, a| Ok(Value::Bool(std::env::set_current_dir(&*arg(&a, 0).to_str()).is_ok()))),
        ("tempnam", |_, a| {
            let dir = arg(&a, 0).to_str();
            let dir = if std::path::Path::new(&*dir).is_dir() { dir.to_string() } else { "/tmp".to_string() };
            let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
            for i in 0..1000u32 {
                let p = format!("{dir}/{}{:06x}", arg(&a, 1).to_str(), (n.wrapping_add(i * 7919)) & 0xffffff);
                if std::fs::OpenOptions::new().write(true).create_new(true).open(&p).is_ok() {
                    return Ok(s(&p));
                }
            }
            Ok(Value::Bool(false))
        }),
        ("scandir", |_, a| {
            let Ok(rd) = std::fs::read_dir(&*arg(&a, 0).to_str()) else { return Ok(Value::Bool(false)) };
            let mut names: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
            names.push(".".into());
            names.push("..".into());
            names.sort();
            if arg(&a, 1).to_int() == 1 {
                names.reverse();
            }
            Ok(arr(names.iter().map(|n| s(n)).collect()))
        }),
        ("is_resource", |_, a| Ok(Value::Bool(matches!(arg(&a, 0), Value::Resource(r) if !r.is_closed())))),
        ("get_resource_type", |_, a| Ok(match arg(&a, 0) {
            Value::Resource(r) => s(if r.is_closed() { "Unknown" } else { r.kind }),
            _ => Value::Bool(false),
        })),
        ("get_resource_id", |_, a| Ok(Value::Int(match arg(&a, 0) {
            Value::Resource(r) => r.id,
            _ => 0,
        }))),
        ("str_getcsv", |_, a| {
            let d = a.get(1).and_then(|v| v.to_str().chars().next()).unwrap_or(',');
            let e = a.get(2).and_then(|v| v.to_str().chars().next()).unwrap_or('"');
            Ok(arr(csv_parse(&arg(&a, 0).to_str(), d, e)))
        }),
        ("parse_url", |_, a| {
            let parts = parse_url(&arg(&a, 0).to_str());
            Ok(match a.get(1).map(Value::to_int).filter(|c| *c >= 0) {
                Some(c) => {
                    let key = ["scheme", "host", "port", "user", "pass", "path", "query", "fragment"].get(c as usize).copied().unwrap_or("");
                    parts.get(&Key::from_str(key)).cloned().unwrap_or(Value::Null)
                }
                None => Value::Array(Rc::new(parts)),
            })
        }),
    ];
    for (k, f) in list {
        m.insert(k, *f);
    }
}

pub fn register_vm(m: &mut HashMap<&'static str, VmBuiltin>) {
    let list: &[(&str, VmBuiltin)] = &[
        ("fopen", |vm, a| {
            let (path, mode) = (arg(&a, 0).to_str(), arg(&a, 1).to_str());
            if let Some(scheme) = path.split_once("://").map(|(s, _)| s.to_ascii_lowercase()) {
                if let Some(class_name) = vm.stream_wrappers.get(&scheme).cloned() {
                    let class = vm.find_class(&class_name)?;
                    let obj = vm.instantiate(&class, vec![])?;
                    let opened = vm.call_method_named(&obj, "stream_open", vec![Value::str(&path), Value::str(&mode), Value::Int(0), Value::Null])?;
                    if !opened.to_bool() {
                        return Ok(Value::Bool(false));
                    }
                    return Ok(Value::Resource(Resource::new(crate::stream::Stream::User { obj }, "stream")));
                }
            }
            Ok(match Stream::open(&path, &mode) {
                Some(st) => Value::Resource(Resource::new(st, "stream")),
                None => Value::Bool(false),
            })
        }),
        ("tmpfile", |_, _| {
            let p = format!("/tmp/php{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0) & 0xffffff);
            let st = Stream::open(&p, "w+");
            let _ = std::fs::remove_file(&p);
            Ok(st.map(|s| Value::Resource(Resource::new(s, "stream"))).unwrap_or(Value::Bool(false)))
        }),
        ("fclose", |vm, a| {
            let Some(r) = res(&a, 0) else { return bad_resource(vm, "fclose", &a) };
            if r.is_closed() {
                return vm.throw_error("TypeError", "fclose(): supplied resource is not a valid stream resource");
            }
            *r.stream.borrow_mut() = None;
            ok(true)
        }),
        ("fwrite", |vm, a| {
            let data = arg(&a, 1).to_str();
            let len = a.get(2).filter(|v| !matches!(v, Value::Null)).map(|v| v.to_int().max(0) as usize).unwrap_or(data.len()).min(data.len());
            with_stream(vm, "fwrite", &a, |st, vm| match st {
                Stream::Stdout => {
                    vm.out.push_str(&data[..len]);
                    Value::Int(len as i64)
                }
                Stream::Output => {
                    vm.write(&data[..len]);
                    Value::Int(len as i64)
                }
                other => other.write(&data.as_bytes()[..len]).map(|n| Value::Int(n as i64)).unwrap_or(Value::Bool(false)),
            })
        }),
        ("fputs", |vm, a| vm_call(vm, "fwrite", a)),
        ("fread", |vm, a| {
            let n = arg(&a, 1).to_int();
            if n <= 0 {
                return vm.throw_error("ValueError", "fread(): Argument #2 ($length) must be greater than 0");
            }
            with_stream(vm, "fread", &a, |st, _| lossy(&st.read(n as usize)))
        }),
        ("fgets", |vm, a| {
            let max = a.get(1).map(|v| (v.to_int().max(1) - 1) as usize).unwrap_or(usize::MAX);
            with_stream(vm, "fgets", &a, |st, _| st.read_line(max).map(|l| lossy(&l)).unwrap_or(Value::Bool(false)))
        }),
        ("fgetc", |vm, a| with_stream(vm, "fgetc", &a, |st, _| st.read(1).first().map(|c| lossy(&[*c])).unwrap_or(Value::Bool(false)))),
        ("feof", |vm, a| with_stream(vm, "feof", &a, |st, _| Value::Bool(st.eof()))),
        ("fflush", |vm, a| with_stream(vm, "fflush", &a, |_, _| Value::Bool(true))),
        ("flock", |_, _| ok(true)),
        ("ftell", |vm, a| with_stream(vm, "ftell", &a, |st, _| Value::Int(st.tell()))),
        ("fseek", |vm, a| {
            let (off, wh) = (arg(&a, 1).to_int(), arg(&a, 2).to_int());
            with_stream(vm, "fseek", &a, |st, _| Value::Int(if st.seek(off, wh) { 0 } else { -1 }))
        }),
        ("rewind", |vm, a| with_stream(vm, "rewind", &a, |st, _| Value::Bool(st.seek(0, 0)))),
        ("ftruncate", |vm, a| {
            let n = arg(&a, 1).to_int().max(0) as usize;
            with_stream(vm, "ftruncate", &a, |st, _| Value::Bool(st.truncate(n)))
        }),
        ("fstat", |vm, a| {
            with_stream(vm, "fstat", &a, |st, _| {
                let mut out = PhpArray::new();
                out.set(Key::from_str("size"), Value::Int(st.size().unwrap_or(0) as i64));
                Value::Array(Rc::new(out))
            })
        }),
        ("stream_get_contents", |vm, a| with_stream(vm, "stream_get_contents", &a, |st, _| lossy(&st.read(usize::MAX / 2)))),
        ("stream_get_meta_data", |vm, a| {
            with_stream(vm, "stream_get_meta_data", &a, |st, _| {
                let mut out = PhpArray::new();
                let (mode, uri, kind) = match st {
                    Stream::File { mode, path, .. } => (mode.clone(), path.clone(), "STDIO"),
                    Stream::Memory { .. } => ("w+b".into(), "php://memory".into(), "MEMORY"),
                    _ => ("r".into(), "php://stdin".into(), "STDIO"),
                };
                out.set(Key::from_str("stream_type"), s(kind));
                out.set(Key::from_str("mode"), s(&mode));
                out.set(Key::from_str("uri"), s(&uri));
                out.set(Key::from_str("seekable"), Value::Bool(true));
                out.set(Key::from_str("eof"), Value::Bool(st.eof()));
                Value::Array(Rc::new(out))
            })
        }),
        ("fpassthru", |vm, a| {
            let data = with_stream(vm, "fpassthru", &a, |st, _| st.read(usize::MAX / 2))?;
            vm.write(&String::from_utf8_lossy(&data));
            Ok(Value::Int(data.len() as i64))
        }),
        ("fgetcsv", |vm, a| {
            let d = a.get(2).and_then(|v| v.to_str().chars().next()).unwrap_or(',');
            let e = a.get(3).and_then(|v| v.to_str().chars().next()).unwrap_or('"');
            with_stream(vm, "fgetcsv", &a, |st, _| match st.read_line(usize::MAX) {
                Some(l) => arr(csv_parse(&String::from_utf8_lossy(&l), d, e)),
                None => Value::Bool(false),
            })
        }),
        ("fputcsv", |vm, a| {
            let fields = match arg(&a, 1) {
                Value::Array(x) => x.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>(),
                _ => vec![],
            };
            let line = csv_line(&fields, &a.get(2).map(|v| v.to_str().to_string()).unwrap_or_else(|| ",".into()), &a.get(3).map(|v| v.to_str().to_string()).unwrap_or_else(|| "\"".into()));
            with_stream(vm, "fputcsv", &a, |st, _| st.write(line.as_bytes()).map(|n| Value::Int(n as i64)).unwrap_or(Value::Bool(false)))
        }),
        ("file_get_contents", |vm, a| {
            let path = arg(&a, 0).to_str();
            if &*path == "php://stdin" {
                let mut buf = String::new();
                let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf);
                return Ok(s(&buf));
            }
            let _ = vm;
            Ok(std::fs::read(path.strip_prefix("file://").unwrap_or(&path)).map(|b| lossy(&b)).unwrap_or(Value::Bool(false)))
        }),
        ("file_put_contents", |_, a| {
            let path = arg(&a, 0).to_str();
            let data: Vec<u8> = match arg(&a, 1) {
                Value::Array(x) => x.iter().flat_map(|(_, v)| v.to_str().as_bytes().to_vec()).collect(),
                other => other.to_str().as_bytes().to_vec(),
            };
            let append = arg(&a, 2).to_int() & 8 != 0;
            let mut o = std::fs::OpenOptions::new();
            o.create(true).write(true);
            if append {
                o.append(true);
            } else {
                o.truncate(true);
            }
            Ok(o.open(&*path).and_then(|mut f| f.write_all(&data)).map(|_| Value::Int(data.len() as i64)).unwrap_or(Value::Bool(false)))
        }),
        ("file", |_, a| {
            let flags = arg(&a, 1).to_int();
            let Ok(bytes) = std::fs::read(&*arg(&a, 0).to_str()) else { return Ok(Value::Bool(false)) };
            let text = String::from_utf8_lossy(&bytes).to_string();
            let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
            if flags & 2 != 0 {
                lines = lines.into_iter().map(|l| l.trim_end_matches(['\n', '\r']).to_string()).collect();
            }
            if flags & 4 != 0 {
                lines.retain(|l| !l.is_empty());
            }
            Ok(arr(lines.iter().map(|l| s(l)).collect()))
        }),
        ("readfile", |vm, a| match std::fs::read(&*arg(&a, 0).to_str()) {
            Ok(b) => {
                vm.write(&String::from_utf8_lossy(&b));
                Ok(Value::Int(b.len() as i64))
            }
            Err(_) => Ok(Value::Bool(false)),
        }),
        ("opendir", |_, a| {
            let Ok(rd) = std::fs::read_dir(&*arg(&a, 0).to_str()) else { return Ok(Value::Bool(false)) };
            let mut entries: Vec<String> = vec![".".into(), "..".into()];
            entries.extend(rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()));
            Ok(Value::Resource(Resource::new(Stream::Dir { entries, pos: 0 }, "stream")))
        }),
        ("readdir", |vm, a| {
            with_stream(vm, "readdir", &a, |st, _| match st {
                Stream::Dir { entries, pos } => {
                    let e = entries.get(*pos).cloned();
                    *pos += 1;
                    e.map(|e| s(&e)).unwrap_or(Value::Bool(false))
                }
                _ => Value::Bool(false),
            })
        }),
        ("rewinddir", |vm, a| {
            with_stream(vm, "rewinddir", &a, |st, _| {
                if let Stream::Dir { pos, .. } = st {
                    *pos = 0;
                }
                Value::Null
            })
        }),
        ("closedir", |vm, a| vm_call(vm, "fclose", a)),
        ("stream_get_line", |vm, a| {
            let max = arg(&a, 1).to_int().max(0) as usize;
            let delim = arg(&a, 2).to_str().to_string();
            with_stream(vm, "stream_get_line", &a, |st, _| {
                let mut out: Vec<u8> = Vec::new();
                loop {
                    if max > 0 && out.len() >= max {
                        break;
                    }
                    let b = st.read(1);
                    let Some(&c) = b.first() else { break };
                    out.push(c);
                    if !delim.is_empty() && out.ends_with(delim.as_bytes()) {
                        out.truncate(out.len() - delim.len());
                        break;
                    }
                }
                if out.is_empty() { Value::Bool(false) } else { lossy(&out) }
            })
        }),
        ("stream_copy_to_stream", |vm, a| {
            let Some(dst) = res(&a, 1) else { return bad_resource(vm, "stream_copy_to_stream", &a) };
            let data = with_stream(vm, "stream_copy_to_stream", &a, |st, _| st.read(usize::MAX / 2))?;
            let taken = dst.stream.borrow_mut().take();
            let n = match taken {
                Some(mut st) => {
                    let w = st.write(&data).unwrap_or(0);
                    *dst.stream.borrow_mut() = Some(st);
                    w
                }
                None => 0,
            };
            Ok(Value::Int(n as i64))
        }),
    ];
    for (k, f) in list {
        m.insert(k, *f);
    }
}

/// Forward a stream function on a user-wrapper resource to the wrapper object's methods (streamWrapper API).
pub fn user_stream_op(vm: &mut Vm<'_>, name: &str, r: Rc<Resource>, args: &[Value]) -> Option<R<Value>> {
    let obj = match &*r.stream.borrow() {
        Some(crate::stream::Stream::User { obj }) => obj.clone(),
        _ => return None,
    };
    let call = |vm: &mut Vm<'_>, m: &str, a: Vec<Value>| vm.call_method_named(&obj, m, a);
    let arg_at = |i: usize| args.get(i).cloned().unwrap_or(Value::Null);
    let r_val = match name {
        "fread" => call(vm, "stream_read", vec![Value::Int(arg_at(1).to_int())]).map(|v| if matches!(v, Value::Bool(false)) { Value::str("") } else { v }),
        "fwrite" | "fputs" => {
            let data = arg_at(1).to_str();
            call(vm, "stream_write", vec![Value::Str(data.clone())]).map(|v| if matches!(v, Value::Bool(_) | Value::Null) { Value::Int(data.len() as i64) } else { v })
        }
        "fgets" => {
            let mut out = String::new();
            loop {
                let c = match call(vm, "stream_read", vec![Value::Int(1)]) {
                    Ok(v) => v.to_str().to_string(),
                    Err(e) => return Some(Err(e)),
                };
                if c.is_empty() {
                    break;
                }
                out.push_str(&c);
                if c == "\n" {
                    break;
                }
            }
            Ok(if out.is_empty() { Value::Bool(false) } else { Value::str(&out) })
        }
        "feof" => call(vm, "stream_eof", vec![]).map(|v| Value::Bool(v.to_bool())),
        "ftell" => call(vm, "stream_tell", vec![]),
        "fseek" => call(vm, "stream_seek", vec![arg_at(1), Value::Int(if args.len() > 2 { arg_at(2).to_int() } else { 0 })]).map(|v| Value::Int(if v.to_bool() { 0 } else { -1 })),
        "fflush" => call(vm, "stream_flush", vec![]).map(|v| Value::Bool(v.to_bool())),
        "fclose" => {
            let res = call(vm, "stream_close", vec![]).map(|_| Value::Bool(true));
            *r.stream.borrow_mut() = None;
            res
        }
        _ => return None,
    };
    Some(r_val)
}

fn vm_call(vm: &mut Vm<'_>, name: &str, a: Vec<Value>) -> R<Value> {
    match crate::builtins::call(vm, name, a) {
        Some(r) => r,
        None => Err(Flow::Fatal(format!("Call to undefined function {name}()"))),
    }
}
