//! PipeWire sink на базе связки `pw-loopback` + `paplay` (как в phonecam).
//!
//! Создаёт виртуальную пару PipeWire узлов:
//! 1. OpenMicSink (Audio/Sink, стерео [FL,FR]) — виртуальный приёмник
//! 2. OpenMic (Audio/Source, стерео [FL,FR]) — виртуальный системный микрофон
//!
//! Смартфон шлёт моно-звук 48 кГц S16LE. Мы дублируем моно в стерео (L=R)
//! и отправляем непрерывным потоком в `paplay -d OpenMicSink`.
//! Это даёт безупречно чистый звук без треска, щелчков и заиканий FIFO.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const SINK_NAME: &str = "OpenMicSink";
const SOURCE_NAME: &str = "OpenMic";

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
        // Моно -> стерео дублирование (2 байта моно -> 4 байта стерео L+R)
        let mut stereo = Vec::with_capacity(pcm_le.len() * 2);
        for chunk in pcm_le.chunks_exact(2) {
            stereo.extend_from_slice(chunk);
            stereo.extend_from_slice(chunk);
        }
        let _ = self.tx.send(stereo);
    }
}

pub fn spawn() -> anyhow::Result<PipewireHandle> {
    ensure_pw_loopback()?;

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::Builder::new()
        .name("openmic-play".into())
        .spawn(move || feeder_loop(rx))?;

    Ok(PipewireHandle { tx })
}

/// Проверяет, запущен ли `pw-loopback` с OpenMicSink, и при необходимости стартует его
fn ensure_pw_loopback() -> anyhow::Result<()> {
    let out = Command::new("pactl").args(["list", "short", "sinks"]).output()?;
    let sinks = String::from_utf8_lossy(&out.stdout);
    if sinks.contains(SINK_NAME) {
        return Ok(());
    }

    // Запускаем фоновый процесс pw-loopback
    let capture_props = format!(
        "node.name={SINK_NAME} node.description=\"{SINK_NAME}\" media.class=Audio/Sink audio.position=[FL,FR]"
    );
    let playback_props = format!(
        "node.name={SOURCE_NAME} node.description=\"{SOURCE_NAME}\" media.class=Audio/Source audio.position=[FL,FR]"
    );

    Command::new("pw-loopback")
        .args([
            "--capture-props",
            &capture_props,
            "--playback-props",
            &playback_props,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    // Ждём появления узла в системе
    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(out) = Command::new("pactl").args(["list", "short", "sinks"]).output() {
            if String::from_utf8_lossy(&out.stdout).contains(SINK_NAME) {
                return Ok(());
            }
        }
    }

    anyhow::bail!("PipeWire не создал виртуальный узел {SINK_NAME}");
}

/// Поток подачи звука через paplay в OpenMicSink
fn feeder_loop(rx: mpsc::Receiver<Vec<u8>>) {
    // 20 мс тишины в стерео 48 кГц S16LE (960 фреймов * 2 канала * 2 байта = 3840 байт)
    let silence = [0u8; 3840];

    loop {
        let mut child = match spawn_paplay() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[openmicd] paplay spawn error: {e:#}, retry in 1s");
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };

        let mut stdin = match child.stdin.take() {
            Some(s) => s,
            None => continue,
        };

        loop {
            match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(chunk) => {
                    if stdin.write_all(&chunk).is_err() {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Пока смартфон молчит или переподключается — гоним гладкую тишину
                    if stdin.write_all(&silence).is_err() {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = child.kill();
                    return;
                }
            }
        }

        let _ = child.kill();
        let _ = child.wait();
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn spawn_paplay() -> anyhow::Result<Child> {
    let child = Command::new("paplay")
        .args([
            "-d",
            SINK_NAME,
            "--raw",
            "--format=s16le",
            "--rate=48000",
            "--channels=2",
            "--latency-msec=30",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    Ok(child)
}
