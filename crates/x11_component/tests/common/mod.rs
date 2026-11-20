#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use ed25519_dalek::{Signer as _, SigningKey, VerifyingKey};
use sha2::{Digest as _, Sha256};

pub struct TestDir(PathBuf);

impl TestDir {
    pub fn new(name: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "navop-x11-test-{name}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn public_key(signing: &SigningKey) -> String {
    BASE64.encode(VerifyingKey::from(signing).to_bytes())
}

pub struct ArtifactSpec<'a> {
    pub url: &'a str,
    pub payload: &'a [u8],
    pub signing: &'a SigningKey,
    pub format: &'a str,
    pub executable_path: &'a str,
}

pub fn manifest(url: &str, payload: &[u8], signing: &SigningKey) -> String {
    manifest_artifact(ArtifactSpec {
        url,
        payload,
        signing,
        format: "binary",
        executable_path: "yserver.exe",
    })
}

pub fn manifest_artifact(spec: ArtifactSpec<'_>) -> String {
    let sha = hex::encode(Sha256::digest(spec.payload));
    let signature = BASE64.encode(spec.signing.sign(spec.payload).to_bytes());
    serde_json::json!({
        "schema": 1,
        "components": {
            "x11-server": {
                "channel": "stable",
                "pinned_for_navop": ">=0.16.0, <1.0.0",
                "rollout_percent": 100,
                "artifacts": {
                    "windows-x86_64": {
                        "version": "1.9.0",
                        "url": spec.url,
                        "sha256": sha,
                        "signature": signature,
                        "min_vulkan": "1.3.0",
                        "format": spec.format,
                        "executable_path": spec.executable_path
                    }
                }
            }
        }
    })
    .to_string()
}

#[derive(Default)]
pub struct MockState {
    responses: HashMap<String, (u16, Vec<u8>)>,
    hits: HashMap<String, usize>,
}

pub struct MockServer {
    pub url: String,
    state: Arc<Mutex<MockState>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MockServer {
    pub fn new(routes: &[(&str, u16, &[u8])]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let mut state = MockState::default();
        for (path, status, body) in routes {
            state
                .responses
                .insert((*path).into(), (*status, body.to_vec()));
        }
        let state = Arc::new(Mutex::new(state));
        let worker_state = state.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let thread = std::thread::spawn(move || {
            while !worker_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        // macOS 接受连接可能继承非阻塞标志，测试服务端必须等待完整请求。
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                            .unwrap();
                        serve(stream, &worker_state);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url: format!("http://127.0.0.1:{port}"),
            state,
            stop,
            thread: Some(thread),
        }
    }

    pub fn set_routes(&self, routes: &[(&str, u16, &[u8])]) {
        let mut state = self.state.lock().unwrap();
        state.responses.clear();
        for (path, status, body) in routes {
            state
                .responses
                .insert((*path).into(), (*status, body.to_vec()));
        }
    }

    pub fn hits(&self, path: &str) -> usize {
        self.state
            .lock()
            .unwrap()
            .hits
            .get(path)
            .copied()
            .unwrap_or(0)
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.url.trim_start_matches("http://"));
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

fn serve(mut stream: TcpStream, state: &Arc<Mutex<MockState>>) {
    let mut request = Vec::new();
    let mut buffer = [0; 1024];
    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let Ok(read) = stream.read(&mut buffer) else {
            return;
        };
        if read == 0 {
            return;
        }
        request.extend_from_slice(&buffer[..read]);
    }
    let text = String::from_utf8_lossy(&request);
    let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
    let mut state = state.lock().unwrap();
    *state.hits.entry(path.clone()).or_default() += 1;
    let Some((status, body)) = state.responses.get(&path) else {
        write_response(&mut stream, 404, b"missing");
        return;
    };
    write_response(&mut stream, *status, body);
}

fn write_response(stream: &mut TcpStream, status: u16, body: &[u8]) {
    let header = format!(
        "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
}

pub fn unused_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}/manifest.json")
}
