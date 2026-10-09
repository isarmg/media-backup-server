use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};
const BINARY: &str = env!("CARGO_BIN_EXE_xszs");
const SERVICE: &str = "xszs";
fn private(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn output(config: &Path, args: &[&str]) -> Output {
    Command::new(BINARY)
        .env_clear()
        .args(args)
        .arg("--config")
        .arg(config)
        .arg("--json")
        .output()
        .unwrap()
}
fn report(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout is not one JSON record: {} / {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
fn state(path: &Path) -> BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)> {
    fn collect(
        root: &Path,
        path: &Path,
        result: &mut BTreeMap<PathBuf, (Vec<u8>, std::time::SystemTime)>,
    ) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let meta = entry.metadata().unwrap();
            if meta.is_dir() {
                collect(root, &entry.path(), result)
            } else {
                result.insert(
                    entry.path().strip_prefix(root).unwrap().to_owned(),
                    (fs::read(entry.path()).unwrap(), meta.modified().unwrap()),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    collect(path, path, &mut result);
    result
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn explicit_initialization_read_only_diagnostics_and_real_readiness() {
    let temporary = tempfile::tempdir().unwrap();
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let data = temporary.path().join("data");
    private(&data);
    let shared = temporary.path().join("shared");
    private(&shared);
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let bind = socket.local_addr().unwrap();
    drop(socket);
    let config = temporary.path().join("config.json");
    let mut settings = json!({"data_dir":data,"bind":bind.to_string(),"development":true,"require_https":false,"credentials_key":"BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=","bootstrap_admin_password":"test-password1!"});
    fs::write(&config, serde_json::to_vec(&settings).unwrap()).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let empty = state(&data);
    for flag in ["--help", "--version"] {
        let result = Command::new(BINARY).env_clear().arg(flag).output().unwrap();
        assert!(result.status.success());
        assert_eq!(state(&data), empty);
    }
    for machine in [false, true] {
        let mut command = Command::new(BINARY);
        command
            .env_clear()
            .args(["run", "--bind", "private-cli-secret"]);
        if machine {
            command.arg("--json");
        }
        let rejected = command.output().unwrap();
        assert_eq!(rejected.status.code(), Some(2));
        assert!(!String::from_utf8_lossy(&rejected.stdout).contains("private-cli-secret"));
        assert!(!String::from_utf8_lossy(&rejected.stderr).contains("private-cli-secret"));
        if machine {
            assert_eq!(report(&rejected)["code"], "invalid_cli_input");
        }
    }
    let missing = output(&config, &["run"]);
    assert!(!missing.status.success());
    assert!(report(&missing)["code"].is_string());
    assert_eq!(state(&data), empty, "run initialized an empty directory");
    let invalid = output(&config, &["nonsense"]);
    assert_eq!(invalid.status.code(), Some(2));
    assert!(report(&invalid)["code"].is_string());
    assert_eq!(state(&data), empty);
    let initialized = output(&config, &["init"]);
    assert!(
        initialized.status.success(),
        "{}",
        format!(
            "stdout={} stderr={}",
            String::from_utf8_lossy(&initialized.stdout),
            String::from_utf8_lossy(&initialized.stderr)
        )
    );
    assert_eq!(report(&initialized)["ready"], false);
    let initialized_state = state(&data);
    assert!(!initialized_state.is_empty());
    let again = output(&config, &["init"]);
    assert!(!again.status.success());
    report(&again);
    assert_eq!(
        state(&data),
        initialized_state,
        "repeated init changed current data"
    );
    let parent_traversal = output(
        &temporary.path().join("data/../config.json"),
        &["config", "validate"],
    );
    assert!(!parent_traversal.status.success());
    report(&parent_traversal);
    assert_eq!(state(&data), initialized_state);
    let valid = output(&config, &["config", "validate"]);
    assert!(
        valid.status.success(),
        "{}",
        String::from_utf8_lossy(&valid.stderr)
    );
    let valid_report = report(&valid);
    assert!(valid_report["schema_identity"].is_object());
    assert!(
        valid_report["state_paths"]
            .as_array()
            .unwrap()
            .iter()
            .all(|path| Path::new(path.as_str().unwrap()).is_absolute())
    );
    assert_eq!(
        state(&data),
        initialized_state,
        "validation changed bytes or timestamps"
    );
    let stopped = output(&config, &["status"]);
    assert!(!stopped.status.success());
    report(&stopped);
    assert_eq!(
        state(&data),
        initialized_state,
        "offline status changed state"
    );
    let pending = data.join(".xcss-maintenance-pending.json");
    fs::write(&pending, b"{}").unwrap();
    fs::set_permissions(&pending, fs::Permissions::from_mode(0o600)).unwrap();
    let pending_state = state(&data);
    for command in ["run", "init"] {
        let rejected = output(&config, &[command]);
        assert!(!rejected.status.success());
        assert_eq!(report(&rejected)["code"], "state.maintenance_pending");
        assert_eq!(state(&data), pending_state);
    }
    assert!(output(&config, &["config", "validate"]).status.success());
    assert_eq!(state(&data), pending_state);
    fs::remove_file(&pending).unwrap();
    let original = fs::read(&config).unwrap();
    settings["unexpected_private_field"] = json!("never-emit-this-secret");
    fs::write(&config, serde_json::to_vec(&settings).unwrap()).unwrap();
    let unknown = output(&config, &["config", "validate"]);
    assert!(!unknown.status.success());
    report(&unknown);
    assert!(!String::from_utf8_lossy(&unknown.stdout).contains("never-emit-this-secret"));
    assert_eq!(state(&data), initialized_state);
    fs::write(&config, original).unwrap();
    let child = Command::new(BINARY)
        .env_clear()
        .arg("run")
        .arg("--config")
        .arg(&config)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut running = Running(child);
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let status = output(&config, &["status"]);
        if status.status.success() {
            assert_eq!(report(&status)["service"], SERVICE);
            assert_eq!(report(&status)["ready"], true);
            break;
        }
        assert!(
            running.0.try_wait().unwrap().is_none(),
            "server exited before readiness: {}",
            String::from_utf8_lossy(&status.stdout)
        );
        assert!(
            Instant::now() < deadline,
            "service readiness deadline exceeded"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let logged_request =
        assert_http_error(bind, "GET", "/api/v1/nonexistent-contract-route", "", 404);
    assert_http_error(bind, "POST", "/api/v1/auth/login", "{", 400);
    assert_http_error(bind, "GET", "/api/v1/auth/login", "", 405);
    assert_http_error(bind, "POST", "/api/v1/auth/login", &"x".repeat(70_000), 413);
    assert_http_error(bind, "POST", "/v1/auth/bootstrap", "{", 400);
    let log_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let correlated = fs::read_dir(data.join("logs"))
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| {
                fs::read_to_string(entry.path())
                    .unwrap_or_default()
                    .lines()
                    .any(|line| {
                        serde_json::from_str::<Value>(line)
                            .ok()
                            .is_some_and(|record| {
                                record["request_id"] == logged_request
                                    && record["service"] == SERVICE
                            })
                    })
            });
        if correlated {
            break;
        }
        assert!(
            Instant::now() < log_deadline,
            "HTTP log did not correlate with response request ID"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let while_running = output(&config, &["config", "validate"]);
    assert!(
        while_running.status.success(),
        "{}",
        String::from_utf8_lossy(&while_running.stdout)
    );
    let second_socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let second_bind = second_socket.local_addr().unwrap();
    drop(second_socket);
    let second = output(&config, &["run", "--bind", &second_bind.to_string()]);
    assert!(!second.status.success());
    report(&second);
    assert!(output(&config, &["status"]).status.success());
    assert!(data.join("logs").is_dir());
    assert!(
        Command::new("kill")
            .args(["-TERM", &running.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    assert!(running.0.wait().unwrap().success());
    let records = fs::read_dir(data.join("logs"))
        .unwrap()
        .filter_map(Result::ok)
        .flat_map(|entry| {
            fs::read_to_string(entry.path())
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
        .collect::<Vec<_>>();
    for event in [
        "common.config.loaded",
        "common.runtime.started",
        "common.runtime.shutdown_started",
        "common.runtime.stopped",
    ] {
        assert!(
            records
                .iter()
                .any(|record| record["event"] == event && record["service"] == SERVICE),
            "missing common event {event}: {records:?}"
        );
    }
}

fn assert_http_error(
    bind: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: &str,
    status: u16,
) -> String {
    use std::io::{Read, Write};
    let mut connection = std::net::TcpStream::connect(bind).unwrap();
    connection
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(connection, "{method} {path} HTTP/1.1\r\nHost: {bind}\r\nOrigin: http://{bind}\r\nSec-Fetch-Site: same-origin\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    let mut response = Vec::new();
    connection.read_to_end(&mut response).unwrap();
    let boundary = response
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .unwrap();
    let head = String::from_utf8_lossy(&response[..boundary]);
    assert!(
        head.lines()
            .next()
            .unwrap()
            .contains(&format!(" {status} ")),
        "{head}"
    );
    let headers = head
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        headers.get("cache-control").map(String::as_str),
        Some("no-store")
    );
    let request_id = headers
        .get("x-request-id")
        .expect("error response has no request ID");
    let mut payload = response[boundary + 4..].to_vec();
    if headers
        .get("transfer-encoding")
        .is_some_and(|value| value == "chunked")
    {
        let mut decoded = Vec::new();
        let mut cursor = 0;
        loop {
            let end = payload[cursor..]
                .windows(2)
                .position(|bytes| bytes == b"\r\n")
                .unwrap()
                + cursor;
            let length = usize::from_str_radix(
                std::str::from_utf8(&payload[cursor..end])
                    .unwrap()
                    .split(';')
                    .next()
                    .unwrap(),
                16,
            )
            .unwrap();
            if length == 0 {
                break;
            }
            cursor = end + 2;
            decoded.extend_from_slice(&payload[cursor..cursor + length]);
            cursor += length + 2;
        }
        payload = decoded;
    }
    let record: Value = serde_json::from_slice(&payload).unwrap_or_else(|_| {
        panic!(
            "non JSON error: {} / {head}",
            String::from_utf8_lossy(&payload)
        )
    });
    assert!(record["code"].is_string(), "{record}");
    assert_eq!(record["request_id"].as_str(), Some(request_id.as_str()));
    request_id.clone()
}
