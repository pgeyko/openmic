//! OpenMic wire protocol v1 (см. docs/TZ.md §5).
//!
//! Транспорт: одно TCP-соединение.
//! 1. Handshake: JSON-строка с `\n` в обе стороны.
//! 2. Дальше C→S: бинарные аудио-фреймы `[u32 LE len][i16 LE PCM...]`.
//!    S→C: JSON-строки управления с `\n`.
//! Детект JSON vs бинарь на приёме: аудио-фрейм всегда <= 8192 байт,
//! а первые 4 байта JSON (`{"ty` и т.п.) как u32 LE дают гигантское число.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

pub mod local;

pub const DEFAULT_PORT: u16 = 59100;
/// Максимальный размер одного аудио-фрейма (20 мс при 48к = 1920 байт, берём с запасом).
pub const MAX_FRAME_BYTES: u32 = 8192;
/// Всё, что больше — считаем началом JSON-строки управления.
pub const JSON_DETECT_THRESHOLD: u32 = 1_000_000;

// --- handshake ---

#[derive(Debug, Serialize, Deserialize)]
pub struct Hello {
    pub hello: String,
    pub token: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Welcome {
    pub ok: bool,
    #[serde(default)]
    pub sample_rate: u32,
    #[serde(default)]
    pub channels: u8,
    #[serde(default)]
    pub error: String,
}

impl Hello {
    pub fn new(token: &str) -> Self {
        Self {
            hello: "openmic/1".into(),
            token: token.into(),
        }
    }
}

// --- control (S→C и C→S после handshake, JSON-строки) ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Control {
    Gain { value: u16 }, // 0..=200, %
    Mute { value: bool },
    Ping,
    Pong,
}

/// Прочитать одну JSON-строку (до `\n`), распарсить.
pub async fn read_json_line<R, T>(reader: &mut R) -> Result<T>
where
    R: AsyncBufReadExt + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let mut line = String::new();
    let n = reader.read_line(&mut line).await?;
    if n == 0 {
        return Err(anyhow!("eof"));
    }
    serde_json::from_str(line.trim()).context("bad json line")
}

/// Записать JSON-строку с `\n`.
pub async fn write_json_line<W, T>(w: &mut W, v: &T) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
    T: Serialize,
{
    let mut s = serde_json::to_string(v)?;
    s.push('\n');
    w.write_all(s.as_bytes()).await?;
    w.flush().await?;
    Ok(())
}

/// Прочитать один аудио-фрейм: `[u32 len][pcm]`, вернуть сырые байты PCM.
pub async fn read_audio_frame<R>(r: &mut R, buf: &mut Vec<u8>) -> Result<usize>
where
    R: AsyncReadExt + Unpin,
{
    let len = r.read_u32_le().await.context("eof/frame len")?;
    if len > MAX_FRAME_BYTES {
        return Err(anyhow!("frame too big: {len} (looks like JSON?)"));
    }
    buf.resize(len as usize, 0);
    r.read_exact(buf).await.context("frame body")?;
    Ok(len as usize)
}

/// Записать аудио-фрейм.
pub async fn write_audio_frame<W>(w: &mut W, pcm: &[u8]) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    if pcm.len() > MAX_FRAME_BYTES as usize {
        return Err(anyhow!("pcm too big"));
    }
    w.write_u32_le(pcm.len() as u32).await?;
    w.write_all(pcm).await?;
    Ok(())
}

/// RMS 16-битного LE PCM в диапазоне 0.0..=1.0. Пустой буфер → 0.
pub fn rms_i16le(pcm: &[u8]) -> f32 {    if pcm.len() < 2 {
        return 0.0;
    }
    let mut sum = 0f64;
    let mut n = 0u32;
    for ch in pcm.chunks_exact(2) {
        let s = i16::from_le_bytes([ch[0], ch[1]]) as f64 / 32768.0;
        sum += s * s;
        n += 1;
    }
    if n == 0 {
        return 0.0;
    }
    (sum / n as f64).sqrt() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_rms_is_zero() {
        assert_eq!(rms_i16le(&[0, 0, 0, 0]), 0.0);
    }

    #[test]
    fn full_scale_rms_near_one() {
        let pcm = [0xFFu8, 0x7F, 0xFF, 0x7F]; // 32767, 32767
        let r = rms_i16le(&pcm);
        assert!(r > 0.99 && r <= 1.0, "{r}");
    }

    #[test]
    fn hello_roundtrip() {
        let h = Hello::new("abcd");
        let s = serde_json::to_string(&h).unwrap();
        let back: Hello = serde_json::from_str(&s).unwrap();
        assert_eq!(back.token, "abcd");
    }
}
