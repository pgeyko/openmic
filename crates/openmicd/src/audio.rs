//! Аудио-параметры демона: формат потока и характеристики фреймов.

pub const SAMPLE_RATE: u32 = 48000;
pub const CHANNELS: u8 = 1;
/// 20 мс при 48 кГц моно S16LE.
#[allow(dead_code)]
pub const FRAME_SAMPLES: usize = 960;
#[allow(dead_code)]
pub const FRAME_BYTES: usize = FRAME_SAMPLES * 2;
