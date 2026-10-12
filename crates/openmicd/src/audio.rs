//! Аудио-выход демона: трейт + заглушки.
//!
//! `PipewireSink` (нода Audio/Source/Virtual "OpenMic") в tranше суб-агента
//! по pipewire-rs; этот модуль фиксирует контракт, чтобы врезка была однострочной:
//! 48 кГц / mono / S16LE, фреймы по 20 мс (960 сэмплов = 1920 байт).

use std::time::{Duration, Instant};

use openmic_proto::rms_i16le;

pub const SAMPLE_RATE: u32 = 48000;
pub const CHANNELS: u8 = 1;
/// 20 мс при 48 кГц моно S16LE.
pub const FRAME_SAMPLES: usize = 960;
pub const FRAME_BYTES: usize = FRAME_SAMPLES * 2;

/// Куда девать звук с телефона.
pub trait AudioSink: Send {
    /// Один фрейм PCM S16LE mono. Вызывается из сетевого таска.
    fn push(&mut self, pcm: &[u8]);
    /// Клиент ушёл: sink должен продолжать отдавать тишину,
    /// чтобы нода PipeWire не пропадала из системы.
    fn idle(&mut self) {}
}

/// Заглушка: считает фреймы/RMS, никуда не выводит.
/// Используется до готовности PipewireSink и в тестах.
pub struct NullSink {
    pub frames: u64,
    pub bytes: u64,
    last_log: Instant,
    peak: f32,
}

impl NullSink {
    pub fn new() -> Self {
        Self {
            frames: 0,
            bytes: 0,
            last_log: Instant::now(),
            peak: 0.0,
        }
    }
}

impl AudioSink for NullSink {
    fn push(&mut self, pcm: &[u8]) {
        self.frames += 1;
        self.bytes += pcm.len() as u64;
        self.peak = self.peak.max(rms_i16le(pcm));
        if self.last_log.elapsed() > Duration::from_secs(2) {
            eprintln!(
                "[openmicd] audio: frames={} bytes={} rms_peak={:.3} (sink=NULL, pipewire TODO)",
                self.frames, self.bytes, self.peak
            );
            self.last_log = Instant::now();
            self.peak = 0.0;
        }
    }
}

/// Тишина одним фреймом 20 мс. Разделяемый буфер, чтобы не аллоцировать в хот-пасе.
pub fn silence_frame() -> [u8; FRAME_BYTES] {
    [0u8; FRAME_BYTES]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_sink_counts() {
        let mut s = NullSink::new();
        s.push(&silence_frame());
        s.push(&[0xFF, 0x7F]);
        assert_eq!(s.frames, 2);
        assert_eq!(s.bytes, (FRAME_BYTES + 2) as u64);
    }
}
