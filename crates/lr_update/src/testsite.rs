//! A fake website for tests: an in-process HTTP server on 127.0.0.1 that holds a tree of files in memory, can publish a
//! signed native build into it exactly like `packaging/native/manifest.mjs` does, and can break things on purpose
//! (404, cut-off bodies, flipped bytes, a dropped connection, going offline).

use crate::manifest::{self, Manifest};
use crate::verify;
use ring::signature::Ed25519KeyPair;
use serde_json::json;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub enum Fault {
    /// Answer with this status instead of the file.
    Status(u16),
    /// Send the headers (full length) and only this many body bytes, then close.
    TruncateAfter(usize),
    /// Flip one byte in the middle of the body.
    FlipByte,
    /// Close the connection without an answer.
    Close,
    /// Send the body in chunked transfer encoding.
    Chunked,
    /// Send more bytes than the headers promise... (plain close-delimited body with no length)
    NoLength,
}

#[derive(Default)]
struct Inner {
    files: HashMap<String, Vec<u8>>,
    faults: HashMap<String, Fault>,
    hits: Vec<String>,
}

pub struct TestSite {
    pub url: String,
    inner: Arc<Mutex<Inner>>,
    stop: Arc<AtomicBool>,
    addr: std::net::SocketAddr,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// One build to publish.
pub struct BuildSpec<'a> {
    pub platform: &'a str,
    pub channel: &'a str,
    pub build: u64,
    /// (path, bytes, mode)
    pub files: Vec<(&'a str, Vec<u8>, u32)>,
    pub run: &'a str,
    pub min_os: Option<&'a str>,
    pub launcher_min: u64,
    pub key: &'a Ed25519KeyPair,
}

impl<'a> BuildSpec<'a> {
    /// One program file `bin/game` of the given bytes, signed with `key`.
    pub fn simple(platform: &'a str, build: u64, program: Vec<u8>, key: &'a Ed25519KeyPair) -> BuildSpec<'a> {
        BuildSpec {
            platform,
            channel: "",
            build,
            files: vec![("bin/game", program, 0o755), ("data/readme.txt", format!("build {build}\n").into_bytes(), 0o644)],
            run: "bin/game",
            min_os: None,
            launcher_min: 1,
            key,
        }
    }
}

fn handle(mut stream: TcpStream, inner: &Mutex<Inner>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
        if buf.len() > 16 * 1024 {
            return;
        }
    }
    let text = String::from_utf8_lossy(&buf).to_string();
    let path = text.lines().next().unwrap_or("").split(' ').nth(1).unwrap_or("/").trim_start_matches('/').split('?').next().unwrap_or("").to_string();
    let (body, fault) = {
        let mut g = inner.lock().unwrap();
        g.hits.push(path.clone());
        (g.files.get(&path).cloned(), g.faults.get(&path).cloned())
    };
    let head = |status: &str, extra: &str| format!("HTTP/1.1 {status}\r\n{extra}Connection: close\r\n\r\n");
    match (body, fault) {
        (_, Some(Fault::Close)) => {
            let _ = stream.shutdown(Shutdown::Both);
        }
        (_, Some(Fault::Status(code))) => {
            let _ = stream.write_all(head(&format!("{code} Error"), "Content-Length: 0\r\n").as_bytes());
        }
        (None, _) => {
            let _ = stream.write_all(head("404 Not Found", "Content-Length: 9\r\n").as_bytes());
            let _ = stream.write_all(b"not found");
        }
        (Some(mut data), fault) => match fault {
            Some(Fault::TruncateAfter(n)) => {
                let _ = stream.write_all(head("200 OK", &format!("Content-Length: {}\r\n", data.len())).as_bytes());
                let _ = stream.write_all(&data[..n.min(data.len())]);
                let _ = stream.flush();
                let _ = stream.shutdown(Shutdown::Both);
            }
            Some(Fault::Chunked) => {
                let _ = stream.write_all(head("200 OK", "Transfer-Encoding: chunked\r\n").as_bytes());
                for part in data.chunks(1000) {
                    let _ = stream.write_all(format!("{:x}\r\n", part.len()).as_bytes());
                    let _ = stream.write_all(part);
                    let _ = stream.write_all(b"\r\n");
                }
                let _ = stream.write_all(b"0\r\n\r\n");
            }
            Some(Fault::NoLength) => {
                let _ = stream.write_all(head("200 OK", "").as_bytes());
                let _ = stream.write_all(&data);
            }
            other => {
                if matches!(other, Some(Fault::FlipByte)) && !data.is_empty() {
                    let mid = data.len() / 2;
                    data[mid] ^= 0xff;
                }
                let _ = stream.write_all(head("200 OK", &format!("Content-Length: {}\r\nContent-Type: application/octet-stream\r\n", data.len())).as_bytes());
                let _ = stream.write_all(&data);
            }
        },
    }
}

impl TestSite {
    pub fn start() -> TestSite {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a test port");
        let addr = listener.local_addr().unwrap();
        let inner = Arc::new(Mutex::new(Inner::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (i2, s2) = (inner.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if s2.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(stream) = stream {
                    let i3 = i2.clone();
                    std::thread::spawn(move || handle(stream, &i3));
                }
            }
        });
        TestSite { url: format!("http://127.0.0.1:{}", addr.port()), inner, stop, addr, thread: Some(thread) }
    }

    pub fn put(&self, path: &str, bytes: Vec<u8>) {
        self.inner.lock().unwrap().files.insert(path.to_string(), bytes);
    }

    pub fn get(&self, path: &str) -> Option<Vec<u8>> {
        self.inner.lock().unwrap().files.get(path).cloned()
    }

    pub fn remove(&self, path: &str) {
        self.inner.lock().unwrap().files.remove(path);
    }

    pub fn fault(&self, path: &str, fault: Fault) {
        self.inner.lock().unwrap().faults.insert(path.to_string(), fault);
    }

    pub fn clear_faults(&self) {
        self.inner.lock().unwrap().faults.clear();
    }

    /// The paths requested so far, in order.
    pub fn hits(&self) -> Vec<String> {
        self.inner.lock().unwrap().hits.clone()
    }

    pub fn clear_hits(&self) {
        self.inner.lock().unwrap().hits.clear();
    }

    /// Stops the server: later connections are refused, which a client sees as being offline.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    /// Loads every file under `dir` into the site below `prefix` (a tree made by packaging/native/manifest.mjs).
    pub fn load_dir(&self, prefix: &str, dir: &std::path::Path) {
        fn walk(site: &TestSite, base: &std::path::Path, here: &std::path::Path, prefix: &str) {
            for e in std::fs::read_dir(here).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(site, base, &p, prefix);
                } else {
                    let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                    site.put(&format!("{prefix}/{rel}"), std::fs::read(&p).unwrap());
                }
            }
        }
        walk(self, dir, dir, prefix.trim_end_matches('/'));
    }

    /// Puts a manifest the caller wrote (possibly hostile) on the site, signed with `key`, and points latest.json at it.
    pub fn publish_manifest_text(&self, platform: &str, channel: &str, build: u64, text: &str, key: &Ed25519KeyPair) {
        let root = format!("{}/{}", manifest::site_root(channel), platform);
        let dir = format!("{root}/builds/{build}");
        let sig = verify::to_hex(key.sign(text.as_bytes()).as_ref());
        self.put(&format!("{dir}/manifest.json"), text.as_bytes().to_vec());
        self.put(&format!("{dir}/manifest.json.sig"), format!("{sig}
").into_bytes());
        let latest = json!({"build": build, "manifest": format!("builds/{build}/manifest.json"), "sha256": verify::sha256_hex(text.as_bytes())});
        self.put(&format!("{root}/latest.json"), latest.to_string().into_bytes());
    }

    /// Publishes a signed build the way `manifest.mjs` does: compressed files, manifest, signature, then latest.json last.
    pub fn publish(&self, spec: &BuildSpec) -> Manifest {
        let root = format!("{}/{}", manifest::site_root(spec.channel), spec.platform);
        let mut entries = Vec::new();
        for (i, (path, bytes, mode)) in spec.files.iter().enumerate() {
            let z = miniz_oxide::deflate::compress_to_vec_zlib(bytes, 6);
            let name = format!("f{i}.z");
            entries.push(json!({
                "path": path, "size": bytes.len(), "sha256": verify::sha256_hex(bytes),
                "z": name, "z_size": z.len(), "z_sha256": verify::sha256_hex(&z),
                "mode": format!("{mode:o}"),
            }));
            self.put(&format!("{root}/builds/{}/{name}", spec.build), z);
        }
        let manifest = json!({
            "schema": "lr.native/1", "platform": spec.platform, "build": spec.build, "version": "0.1.0",
            "revision": format!("{:040x}", spec.build), "built": "2026-11-07T10:00:00Z",
            "min_os": spec.min_os, "launcher_min": spec.launcher_min, "web_build": null, "notes": "test build",
            "run": spec.run, "files": entries,
        });
        let text = serde_json::to_string_pretty(&manifest).unwrap().into_bytes();
        let sig = verify::to_hex(spec.key.sign(&text).as_ref());
        let dir = format!("{root}/builds/{}", spec.build);
        let parsed = Manifest::parse(&text, spec.platform).expect("the test manifest is valid");
        self.put(&format!("{dir}/manifest.json"), text.clone());
        self.put(&format!("{dir}/manifest.json.sig"), format!("{sig}\n").into_bytes());
        let latest = json!({"build": spec.build, "manifest": format!("builds/{}/manifest.json", spec.build), "sha256": verify::sha256_hex(&text)});
        self.put(&format!("{root}/latest.json"), latest.to_string().into_bytes());
        parsed
    }
}

impl Drop for TestSite {
    fn drop(&mut self) {
        self.stop();
    }
}
