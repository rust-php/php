//! Port of php-src/ext/hash (hash.c, hash_adler32.c, hash_fnv.c, hash_joaat.c, hash_crc32.c).
//! Algorithm table and entry points follow hash.c: hash, hash_algos, hash_hmac, hash_init/update/final.
use crate::builtins::{arg, arr, s, VmBuiltin};
use crate::value::*;
use crate::vm::{Flow, Vm};

use std::{collections::HashMap, rc::Rc};

type R<T> = Result<T, Flow>;

use md5::Digest as _;
use sha1::Digest as _;
use sha2::Digest as _;
use sha3::Digest as _;

/// (name, block size for HMAC, digest function). Non-cryptographic hashes use block size 1.
type Algo = (&'static str, usize, fn(&[u8]) -> Vec<u8>);

fn md5(d: &[u8]) -> Vec<u8> {
    md5::Md5::digest(d).to_vec()
}
fn sha1(d: &[u8]) -> Vec<u8> {
    sha1::Sha1::digest(d).to_vec()
}
fn sha224(d: &[u8]) -> Vec<u8> {
    sha2::Sha224::digest(d).to_vec()
}
fn sha256(d: &[u8]) -> Vec<u8> {
    sha2::Sha256::digest(d).to_vec()
}
fn sha384(d: &[u8]) -> Vec<u8> {
    sha2::Sha384::digest(d).to_vec()
}
fn sha512(d: &[u8]) -> Vec<u8> {
    sha2::Sha512::digest(d).to_vec()
}
fn sha3_224(d: &[u8]) -> Vec<u8> {
    sha3::Sha3_224::digest(d).to_vec()
}
fn sha3_256(d: &[u8]) -> Vec<u8> {
    sha3::Sha3_256::digest(d).to_vec()
}
fn sha3_384(d: &[u8]) -> Vec<u8> {
    sha3::Sha3_384::digest(d).to_vec()
}
fn sha3_512(d: &[u8]) -> Vec<u8> {
    sha3::Sha3_512::digest(d).to_vec()
}
fn crc32b(d: &[u8]) -> Vec<u8> {
    crc32fast::hash(d).to_be_bytes().to_vec()
}
/// hash_adler32.c: a = 1 + sum(bytes), b = sum of a; output big-endian b then a.
fn adler32(d: &[u8]) -> Vec<u8> {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in d {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    ((b << 16) | a).to_be_bytes().to_vec()
}
/// hash_fnv.c FNV-1 / FNV-1a, 32 and 64 bit.
fn fnv1_32(d: &[u8]) -> Vec<u8> {
    let mut h: u32 = 0x811c9dc5;
    for &x in d {
        h = h.wrapping_mul(0x01000193) ^ x as u32;
    }
    h.to_be_bytes().to_vec()
}
fn fnv1a_32(d: &[u8]) -> Vec<u8> {
    let mut h: u32 = 0x811c9dc5;
    for &x in d {
        h = (h ^ x as u32).wrapping_mul(0x01000193);
    }
    h.to_be_bytes().to_vec()
}
fn fnv1_64(d: &[u8]) -> Vec<u8> {
    let mut h: u64 = 0xcbf29ce484222325;
    for &x in d {
        h = h.wrapping_mul(0x100000001b3) ^ x as u64;
    }
    h.to_be_bytes().to_vec()
}
fn fnv1a_64(d: &[u8]) -> Vec<u8> {
    let mut h: u64 = 0xcbf29ce484222325;
    for &x in d {
        h = (h ^ x as u64).wrapping_mul(0x100000001b3);
    }
    h.to_be_bytes().to_vec()
}
/// hash_joaat.c: Jenkins one-at-a-time.
fn joaat(d: &[u8]) -> Vec<u8> {
    let mut h: u32 = 0;
    for &x in d {
        h = h.wrapping_add(x as u32);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h = h.wrapping_add(h << 15);
    h.to_be_bytes().to_vec()
}

const ALGOS: &[Algo] = &[
    ("md5", 64, md5),
    ("sha1", 64, sha1),
    ("sha224", 64, sha224),
    ("sha256", 64, sha256),
    ("sha384", 128, sha384),
    ("sha512", 128, sha512),
    ("sha3-224", 144, sha3_224),
    ("sha3-256", 136, sha3_256),
    ("sha3-384", 104, sha3_384),
    ("sha3-512", 72, sha3_512),
    ("crc32b", 1, crc32b),
    ("adler32", 1, adler32),
    ("fnv132", 1, fnv1_32),
    ("fnv1a32", 1, fnv1a_32),
    ("fnv164", 1, fnv1_64),
    ("fnv1a64", 1, fnv1a_64),
    ("joaat", 1, joaat),
];

fn find(name: &str) -> Option<&'static Algo> {
    let l = name.to_ascii_lowercase();
    ALGOS.iter().find(|a| a.0 == l)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// RFC 2104 HMAC, generic over the digest function.
fn hmac(a: &Algo, key: &[u8], msg: &[u8]) -> Vec<u8> {
    let bs = a.1;
    let mut k = if key.len() > bs { (a.2)(key) } else { key.to_vec() };
    k.resize(bs, 0);
    let mut inner: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    inner.extend_from_slice(msg);
    let ih = (a.2)(&inner);
    let mut outer: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    outer.extend_from_slice(&ih);
    (a.2)(&outer)
}

fn bytes_out(digest: Vec<u8>, binary: bool) -> Value {
    if binary {
        s(&String::from_utf8_lossy(&digest))
    } else {
        s(&hex(&digest))
    }
}

fn unknown_algo<T>(vm: &mut Vm<'_>, f: &str, n: usize) -> R<T> {
    vm.throw_error("ValueError", &format!("{f}(): Argument #{n} ($algo) must be a valid hashing algorithm"))
}

/// HashContext: an object holding algorithm name and the data hashed so far.
fn context_props(algo: &str, data: &[u8], hmac_key: Option<&[u8]>) -> PhpArray {
    let mut p = PhpArray::new();
    p.set(Key::from_str("algo"), s(algo));
    p.set(Key::from_str("data"), s(&String::from_utf8_lossy(data)));
    if let Some(k) = hmac_key {
        p.set(Key::from_str("key"), s(&String::from_utf8_lossy(k)));
    }
    p
}

fn context_of(vm: &mut Vm<'_>, v: &Value, f: &str) -> R<(String, Vec<u8>, Option<Vec<u8>>, Rc<crate::object::Obj>)> {
    let Value::Object(o) = v else {
        return vm.throw_error("TypeError", &format!("{f}(): Argument #1 ($context) must be of type HashContext, {} given", v.type_name()));
    };
    let props = o.props.borrow();
    let algo = props.get(&Key::from_str("algo")).map(|x| x.to_str().to_string()).unwrap_or_default();
    let data = props.get(&Key::from_str("data")).map(|x| x.to_str().as_bytes().to_vec()).unwrap_or_default();
    let key = props.get(&Key::from_str("key")).map(|x| x.to_str().as_bytes().to_vec());
    drop(props);
    Ok((algo, data, key, o.clone()))
}

pub fn register_vm(m: &mut HashMap<&'static str, VmBuiltin>) {
    let list: &[(&str, VmBuiltin)] = &[
        ("hash", |vm, a| {
            let Some(al) = find(&arg(&a, 0).to_str()) else { return unknown_algo(vm, "hash", 1) };
            let binary = arg(&a, 2).to_bool();
            Ok(bytes_out((al.2)(arg(&a, 1).to_str().as_bytes()), binary))
        }),
        ("hash_file", |vm, a| {
            let Some(al) = find(&arg(&a, 0).to_str()) else { return unknown_algo(vm, "hash_file", 1) };
            match std::fs::read(&*arg(&a, 1).to_str()) {
                Ok(d) => Ok(bytes_out((al.2)(&d), arg(&a, 2).to_bool())),
                Err(_) => Ok(Value::Bool(false)),
            }
        }),
        ("hash_hmac", |vm, a| {
            let Some(al) = find(&arg(&a, 0).to_str()) else { return unknown_algo(vm, "hash_hmac", 1) };
            if al.0 == "crc32b" || al.0 == "adler32" || al.0.starts_with("fnv") || al.0 == "joaat" {
                return vm.throw_error("ValueError", &format!("hash_hmac(): Argument #1 ($algo) must be a valid cryptographic hashing algorithm"));
            }
            Ok(bytes_out(hmac(al, arg(&a, 2).to_str().as_bytes(), arg(&a, 1).to_str().as_bytes()), arg(&a, 3).to_bool()))
        }),
        ("hash_algos", |_, _| Ok(arr(ALGOS.iter().map(|a| s(a.0)).collect()))),
        ("hash_hmac_algos", |_, _| Ok(arr(ALGOS.iter().filter(|a| a.1 > 1).map(|a| s(a.0)).collect()))),
        ("hash_init", |vm, a| {
            let name = arg(&a, 0).to_str();
            let Some(al) = find(&name) else { return unknown_algo(vm, "hash_init", 1) };
            let key = if arg(&a, 1).to_int() & 1 != 0 { Some(arg(&a, 2).to_str().as_bytes().to_vec()) } else { None };
            let class = vm.find_class("HashContext")?;
            let props = context_props(al.0, &[], key.as_deref());
            Ok(Value::Object(crate::object::Obj::new(class, props, crate::object::Extra::None)))
        }),
        ("hash_update", |vm, a| {
            let (algo, mut data, key, o) = context_of(vm, &arg(&a, 0), "hash_update")?;
            data.extend_from_slice(arg(&a, 1).to_str().as_bytes());
            *o.props.borrow_mut() = context_props(&algo, &data, key.as_deref());
            Ok(Value::Bool(true))
        }),
        ("hash_final", |vm, a| {
            let (algo, data, key, _) = context_of(vm, &arg(&a, 0), "hash_final")?;
            let al = find(&algo).expect("context algo");
            let digest = match key {
                Some(k) => hmac(al, &k, &data),
                None => (al.2)(&data),
            };
            Ok(bytes_out(digest, arg(&a, 1).to_bool()))
        }),
        ("hash_copy", |vm, a| {
            let (algo, data, key, _) = context_of(vm, &arg(&a, 0), "hash_copy")?;
            let class = vm.find_class("HashContext")?;
            Ok(Value::Object(crate::object::Obj::new(class, context_props(&algo, &data, key.as_deref()), crate::object::Extra::None)))
        }),
        ("hash_equals", |vm, a| {
            let Value::Str(known) = arg(&a, 0) else {
                return vm.throw_error("TypeError", &format!("hash_equals(): Argument #1 ($known_string) must be of type string, {} given", arg(&a, 0).type_name()));
            };
            let Value::Str(user) = arg(&a, 1) else {
                return vm.throw_error("TypeError", &format!("hash_equals(): Argument #2 ($user_string) must be of type string, {} given", arg(&a, 1).type_name()));
            };
            Ok(Value::Bool(known.as_bytes() == user.as_bytes()))
        }),
    ];
    for (k, f) in list {
        m.insert(k, *f);
    }
}

/// Plain-function part of hash.c kept for callers that do not need the VM.
pub fn register(_m: &mut HashMap<&'static str, crate::builtins::Builtin>) {}

/// `HashContext` is declared in the PHP prelude; this keeps the class name visible to `hash_init`.
pub const PRELUDE: &str = "final class HashContext { private function __construct() {} }\n";
