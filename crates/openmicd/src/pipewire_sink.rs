//! PipeWire sink через `module-pipe-source` + FIFO.
//!
//! Нативный `pipewire-rs` требует libclang для bindgen (в системе его нет),
//! а `aplay`/`ffmpeg` игнорируют выбор null-sink (проверено: звук уходил мимо).
//! Рабочая схема (проверена сквозным тестом, тон проходит бит-в-бит):
//!
//! ```text
//! openmicd --write--> $XDG_RUNTIME_DIR/openmic.pipe (FIFO, S16LE 48к моно)
//!     --> module-pipe-source "OpenMic" --> Chrome/pavucontrol/Zoom
//! ```
//!
//! Модуль живёт независимо от демона — устройство не пропадает из системы.
//! Пока телефона нет, writer-тред льёт нули (20 мс), модуль не голодает.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::FileTypeExt as _;
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

const SOURCE_NAME: &str = "OpenMic";
/// 20 мс при 48 кГц моно S16LE.
const SILENCE: [u8; 1920] = [0u8; 1920];

/// Ручка из tokio-тасков в writer-тред. Клонируется на каждое соединение.
#[derive(Clone)]
pub struct PipewireHandle {
    tx: mpsc::Sender<Vec<u8>>,
}

impl PipewireHandle {
    pub fn push_bytes(&self, pcm_le: &[u8]) {
        if pcm_le.is_empty() {
            return;
        }
        // best-effort: при переполнении очереди дропаем (не блокируем сеть)
        let _ = self.tx.send(pcm_le.to_vec());
    }
}

pub fn spawn() -> anyhow::Result<PipewireHandle> {
    let fifo = fifo_path();
    ensure_fifo(&fifo)?;
    ensure_module(&fifo)?;
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::Builder::new()
        .name("openmic-pw".into())
        .spawn(move || writer_loop(rx, fifo))?;
    Ok(PipewireHandle { tx })
}

fn fifo_path() -> PathBuf {
    if let Ok(rt) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(rt).join("openmic.pipe");
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "def".into());
    PathBuf::from(format!("/tmp/openmic-{user}.pipe"))
}

fn ensure_fifo(path: &std::path::Path) -> anyhow::Result<()> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_fifo() {
            return Ok(());
        }
        anyhow::bail!("{} существует и это не FIFO", path.display());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // mkfifo через rustix нет в зависимостях — вызываем системный mkfifo.
    let st = Command::new("mkfifo").arg(path).status()?;
    if !st.success() {
        anyhow::bail!("mkfifo {} failed", path.display());
    }
    Ok(())
}

fn pactl(args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("pactl").args(args).output()?;
    if !out.status.success() {
        anyhow::bail!("pactl {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Идемпотентно: грузит module-pipe-source, только если источника ещё нет.
fn ensure_module(fifo: &std::path::Path) -> anyhow::Result<()> {
    let sources = pactl(&["list", "sources", "short"]).unwrap_or_default();
    let has = sources
        .lines()
        .any(|l| l.split_whitespace().nth(1) == Some(SOURCE_NAME));
    if has {
        return Ok(());
    }
    pactl(&[
        "load-module",
        "module-pipe-source",
        &format!("source_name={SOURCE_NAME}"),
        &format!("file={}", fifo.display()),
        "format=s16le",
        "rate=48000",
        "channels=1",
        "source_properties=device.description=OpenMic",
    ])?;
    Ok(())
}

/// Держит FIFO открытым, качает чанки; без данных — тишину.
/// При EPIPE (модуль выгрузили) — пересоздаёт модуль и переоткрывает FIFO.
fn writer_loop(rx: mpsc::Receiver<Vec<u8>>, fifo: PathBuf) {
    loop {
        let mut f = match OpenOptions::new().write(true).open(&fifo) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[openmicd] cannot open {}: {e:#} (retry 1s)", fifo.display());
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        let dead = loop {
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(chunk) => {
                    if f.write_all(&chunk).is_err() {
                        break true;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if f.write_all(&SILENCE).is_err() {
                        break true;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break false,
            }
        };
        if !dead {
            break; // демон завершается
        }
        eprintln!("[openmicd] pipe broken, reloading module");
        if let Err(e) = ensure_module(&fifo) {
            eprintln!("[openmicd] module reload failed: {e:#}");
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}
