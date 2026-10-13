//! PipeWire sink через `module-pipe-source` + неблокирующий FIFO.
//!
//! Модуль живёт независимо от демона — устройство не пропадает из системы.
//! Запись в FIFO ведётся в НЕБЛОКИРУЮЩЕМ режиме: если приложение-потребитель
//! (Chrome, Telegram, Discord) ещё не читает микрофон или притормаживает,
//! writer-тред не зависает наглухо, а аудио-поток не буферизуется пачками,
//! вызывая заикание и рассинхрон.

use std::fs::OpenOptions;
use std::os::unix::fs::{FileTypeExt as _, OpenOptionsExt as _};
use std::os::unix::io::AsRawFd;
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
        // Отправка в writer-тред без блокировки сетевого цикла
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

/// Пишет в FIFO через libc::write с флагом O_NONBLOCK.
/// Если потребитель не читает, либо буфер полон — сэмпл отбрасывается без блокировки,
/// предотвращая заикания и взрывной лаг при возобновлении чтения.
fn nonblocking_write(fd: i32, data: &[u8]) -> bool {
    let res = unsafe { libc::write(fd, data.as_ptr() as *const libc::c_void, data.len()) };
    if res < 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EAGAIN) || err.raw_os_error() == Some(libc::EWOULDBLOCK) {
            // Буфер полон (потребитель временно спит) — это норма, дропаем чанк
            return true;
        }
        // Сломанная труба (EPIPE)
        return false;
    }
    true
}

fn writer_loop(rx: mpsc::Receiver<Vec<u8>>, fifo: PathBuf) {
    loop {
        // Открываем FIFO с флагом O_NONBLOCK
        let file = match OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo)
        {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[openmicd] cannot open {}: {e:#} (retry 1s)", fifo.display());
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };

        let raw_fd = file.as_raw_fd();
        let dead = loop {
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(chunk) => {
                    if !nonblocking_write(raw_fd, &chunk) {
                        break true;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !nonblocking_write(raw_fd, &SILENCE) {
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
