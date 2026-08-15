use dshdesktop_lib::notify::{EventFilter, Notification, NotifySink, NotifySource};
use dshdesktop_lib::notify::ws::WsSource;
use dshdesktop_lib::port::free_port;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

fn system_node() -> PathBuf {
    let out = Command::new("where").arg("node").output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    PathBuf::from(stdout.lines().next().expect("node not found on PATH").trim())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ws_source_receives_filtered_events() {
    let port = free_port().unwrap();
    let work = tempfile::tempdir().unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("fake-dsh.cjs");
    let mut child = Command::new(system_node())
        .arg(fixture)
        .arg("web")
        .arg("--port")
        .arg(port.to_string())
        .current_dir(work.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let (_tx, rx) = watch::channel(Some(port));
    let collected: Arc<Mutex<Vec<Notification>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_store = collected.clone();
    let sink: NotifySink = Arc::new(move |n| sink_store.lock().unwrap().push(n));
    tokio::spawn(Box::new(WsSource { filter: EventFilter::default() }).run(sink, rx));

    let deadline = Instant::now() + Duration::from_secs(15);
    let got = loop {
        if !collected.lock().unwrap().is_empty() {
            break true;
        }
        if Instant::now() > deadline {
            break false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };

    let _ = child.kill();
    assert!(got, "15s 内未收到任何 approval/requested 通知");
    let list = collected.lock().unwrap();
    assert!(list.iter().any(|n| n.body.contains("批准")), "通知正文应可读懂，实际：{:?}", *list);
}
