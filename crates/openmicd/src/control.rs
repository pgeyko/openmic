//! Локальный control-сокет демона: gain/mute/status.
//!
//! Путь: `$XDG_RUNTIME_DIR/openmic.sock` (фолбэк `/tmp/openmic-$USER.sock`).
//! Протокол: одна JSON-строка с `\n` → одна JSON-строка ответа.
//! Запросы: `{"gain":0-200}`, `{"mute":bool}`, `{"status":true}`.

use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicI8, AtomicU32, AtomicU64, AtomicU16, Ordering},
    Arc,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub struct ControlState {
    gain: Arc<AtomicU16>, // %, 0..=200
    mute: Arc<AtomicBool>,
    /// Последний RMS кадра (f32 биты). Читает GUI-watch.
    level: Arc<AtomicU32>,
    /// Телефон в эфире прямо сейчас.
    connected: Arc<AtomicBool>,
    /// Всего принято фреймов за всё время.
    frames: Arc<AtomicU64>,
    /// Желаемый шумодав телефона: -1 = не задано (не трогаем телефон),
    /// 0/1 = выслать клиенту. Меняется с десктопа (CLI/GUI).
    phone_ns: Arc<AtomicI8>,
    phone_seq: Arc<AtomicU64>,
    /// Шина S→C: десктоп → подключённый телефон (JSON-строки).
    phone_tx: broadcast::Sender<PhoneMsg>,
}

/// Сообщение десктоп→телефон с seq для трассировки дублей.
#[derive(Debug, Clone)]
pub struct PhoneMsg {
    pub seq: u64,
    pub line: String,
}

impl ControlState {
    pub fn new() -> Self {
        let (phone_tx, _) = broadcast::channel(16);
        Self {
            gain: Arc::new(AtomicU16::new(100)),
            mute: Arc::new(AtomicBool::new(false)),
            level: Arc::new(AtomicU32::new(0)),
            connected: Arc::new(AtomicBool::new(false)),
            frames: Arc::new(AtomicU64::new(0)),
            phone_ns: Arc::new(AtomicI8::new(-1)),
            phone_seq: Arc::new(AtomicU64::new(0)),
            phone_tx,
        }
    }

    pub fn set_gain(&self, v: u16) {
        self.gain.store(v.min(200), Ordering::Relaxed);
    }

    pub fn set_mute(&self, v: bool) {
        self.mute.store(v, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> (u16, bool) {
        (
            self.gain.load(Ordering::Relaxed),
            self.mute.load(Ordering::Relaxed),
        )
    }

    pub fn push_level(&self, rms: f32) {
        self.level.store(rms.to_bits(), Ordering::Relaxed);
    }

    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    pub fn set_connected(&self, v: bool) {
        self.connected.store(v, Ordering::Relaxed);
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub fn add_frames(&self, n: u64) {
        self.frames.fetch_add(n, Ordering::Relaxed);
    }

    pub fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }

    /// None = не задано; иначе разослать подключённым телефонам.
    pub fn set_phone_ns(&self, v: bool) {
        self.phone_ns.store(if v { 1 } else { 0 }, Ordering::Relaxed);
        let _ = self.phone_tx.send(self.ns_msg(v));
    }

    /// Строка для нового клиента (с seq, как в шине).
    pub fn initial_ns_line(&self) -> Option<String> {
        self.phone_ns().map(|v| self.ns_msg(v).line)
    }

    fn ns_msg(&self, v: bool) -> PhoneMsg {
        let seq = self.phone_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let line = serde_json::json!({"type": "ns", "value": v, "seq": seq}).to_string();
        eprintln!("[openmicd] phone-ctl send seq={seq} ns={v}");
        PhoneMsg { seq, line }
    }

    pub fn phone_ns(&self) -> Option<bool> {
        match self.phone_ns.load(Ordering::Relaxed) {
            1 => Some(true),
            0 => Some(false),
            _ => None,
        }
    }

    pub fn subscribe_phone(&self) -> broadcast::Receiver<PhoneMsg> {
        self.phone_tx.subscribe()
    }

    /// Применить к PCM S16LE in-place. Mute → нули.
    pub fn apply(&self, pcm: &mut [u8]) {
        if self.mute.load(Ordering::Relaxed) {
            pcm.fill(0);
            return;
        }
        let g = self.gain.load(Ordering::Relaxed);
        if g == 100 {
            return;
        }
        for ch in pcm.chunks_exact_mut(2) {
            let s = i16::from_le_bytes([ch[0], ch[1]]) as i32;
            let v = (s * g as i32 / 100).clamp(-32768, 32767) as i16;
            ch.copy_from_slice(&v.to_le_bytes());
        }
    }
}

pub fn socket_path() -> PathBuf {
    if let Ok(rt) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(rt).join("openmic.sock");
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "def".into());
    PathBuf::from(format!("/tmp/openmic-{user}.sock"))
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Req {
    gain: Option<u16>,
    mute: Option<bool>,
    status: Option<bool>,
    /// Шумодав телефона (рассылается подключённым + новым клиентам).
    ns: Option<bool>,
    /// Подписка на уровень: сервер шлёт Level 10 Гц, пока клиент читает.
    watch: Option<bool>,
}

#[derive(Debug, Serialize)]
struct Resp {
    ok: bool,
    gain: u16,
    mute: bool,
    connected: bool,
    frames: u64,
    phone_ns: Option<bool>,
    #[serde(skip_serializing_if = "String::is_empty")]
    error: String,
}

#[derive(Debug, Serialize)]
struct Level {
    level: f32,
    gain: u16,
    mute: bool,
    connected: bool,
    frames: u64,
    phone_ns: Option<bool>,
}

fn snapshot_of(state: &ControlState) -> (Resp, Level) {
    let (g, m) = state.snapshot();
    let resp = Resp {
        ok: true,
        gain: g,
        mute: m,
        connected: state.is_connected(),
        frames: state.frames(),
        phone_ns: state.phone_ns(),
        error: String::new(),
    };
    let lv = Level {
        level: state.level(),
        gain: g,
        mute: m,
        connected: state.is_connected(),
        frames: state.frames(),
        phone_ns: state.phone_ns(),
    };
    (resp, lv)
}

pub async fn serve(state: ControlState) -> Result<()> {
    let path = socket_path();
    let _ = std::fs::remove_file(&path); // stale от прошлого запуска
    let listener = UnixListener::bind(&path).context("bind control sock")?;
    eprintln!("[openmicd] control: {}", path.display());
    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            let (rh, mut wh) = stream.into_split();
            let mut rd = BufReader::new(rh);
            let mut line = String::new();
            loop {
                line.clear();
                let n = match rd.read_line(&mut line).await {
                    Ok(n) => n,
                    Err(_) => break,
                };
                if n == 0 {
                    break;
                }
                let resp = match serde_json::from_str::<Req>(&line) {
                    Ok(req) => {
                        if let Some(g) = req.gain {
                            state.set_gain(g);
                        }
                        if let Some(m) = req.mute {
                            state.set_mute(m);
                        }
                        if let Some(ns) = req.ns {
                            state.set_phone_ns(ns);
                        }
                        if req.watch == Some(true) {
                            // Подписка: 10 Гц шлём уровень, пока клиент читает.
                            loop {
                                let (_, lv) = snapshot_of(&state);
                                let mut s = serde_json::to_string(&lv).unwrap();
                                s.push('\n');
                                if wh.write_all(s.as_bytes()).await.is_err() {
                                    break;
                                }
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            }
                            break;
                        }
                        let (resp, _) = snapshot_of(&state);
                        resp
                    }
                    Err(e) => Resp {
                        ok: false,
                        gain: 0,
                        mute: false,
                        connected: false,
                        frames: 0,
                        phone_ns: None,
                        error: format!("bad request: {e}"),
                    },
                };
                let mut s = serde_json::to_string(&resp).unwrap();
                s.push('\n');
                if wh.write_all(s.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mute_zeroes() {
        let st = ControlState::new();
        st.set_mute(true);
        let mut pcm = vec![0xFF, 0x7F];
        st.apply(&mut pcm);
        assert_eq!(pcm, vec![0, 0]);
    }

    #[test]
    fn gain_scales_and_clamps() {
        let st = ControlState::new();
        st.set_gain(200);
        let mut pcm = 20000i16.to_le_bytes().to_vec();
        st.apply(&mut pcm);
        assert_eq!(i16::from_le_bytes([pcm[0], pcm[1]]), 32767); // clamp
    }
}
