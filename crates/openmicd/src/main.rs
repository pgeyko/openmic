//! openmicd — приём PCM по TCP, отдача в виртуальный микрофон PipeWire.
//!
//! Цепочка: handshake с токеном → приём аудио-фреймов → FIFO в PW-тред →
//! нода `Audio/Source/Virtual "OpenMic"`. Без телефона колбэк льёт нули,
//! устройство не пропадает из системы.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use openmic_proto::{read_audio_frame, read_json_line, rms_i16le, write_json_line};
use openmic_proto::{Hello, Welcome, DEFAULT_PORT};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

mod audio;
mod control;
mod pipewire_sink;

#[derive(Parser, Debug)]
#[command(name = "openmicd", about = "OpenMic daemon: phone mic -> this PC")]
struct Args {
    /// Порт приёма
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,
    /// Файл токена (если нет — генерируется)
    #[arg(long)]
    token_file: Option<PathBuf>,
}

fn token_path(custom: Option<PathBuf>) -> PathBuf {
    custom.unwrap_or_else(|| {
        let base = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let mut h = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()));
                h.push(".config");
                h
            });
        base.join("openmic/token")
    })
}

fn load_or_create_token(path: &std::path::Path) -> Result<String> {
    if path.exists() {
        let t = std::fs::read_to_string(path)?.trim().to_string();
        if !t.is_empty() {
            return Ok(t);
        }
    }
    // 128 бит hex без лишних зависимостей: /dev/urandom.
    let mut b = [0u8; 16];
    let mut f = std::fs::File::open("/dev/urandom")?;
    use std::io::Read as _;
    f.read_exact(&mut b)?;
    let token: String = b.iter().map(|x| format!("{x:02x}")).collect();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, format!("{token}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(token)
}

async fn handle_client(
    stream: TcpStream,
    expected_token: &str,
    pw: pipewire_sink::PipewireHandle,
    ctl: control::ControlState,
) -> Result<()> {
    let peer = stream.peer_addr().ok();
    let (rh, wh) = stream.into_split();
    let mut rd = BufReader::new(rh);
    let wh = std::sync::Arc::new(tokio::sync::Mutex::new(wh));

    let hello: Hello = read_json_line(&mut rd).await.context("handshake read")?;
    if hello.hello != "openmic/1" || hello.token != expected_token {
        let deny = Welcome {
            ok: false,
            sample_rate: 0,
            channels: 0,
            error: "bad-token".into(),
        };
        {
            let mut w = wh.lock().await;
            write_json_line(&mut *w, &deny).await.ok();
            w.shutdown().await.ok();
        }
        anyhow::bail!("bad token from {peer:?}");
    }
    let welcome = Welcome {
        ok: true,
        sample_rate: audio::SAMPLE_RATE,
        channels: audio::CHANNELS,
        error: String::new(),
    };
    {
        let mut w = wh.lock().await;
        write_json_line(&mut *w, &welcome).await.context("welcome write")?;
        // Желаемый шумодав (если задан с десктопа) — сразу телефону.
        if let Some(line) = ctl.initial_ns_line() {
            use tokio::io::AsyncWriteExt as _;
            w.write_all(line.as_bytes()).await.ok();
            w.write_all(b"\n").await.ok();
        }
    }
    eprintln!("[openmicd] client connected: {peer:?}");
    ctl.set_connected(true);

    // Форвардер десктоп→телефон (gain/mute не шлём — они локальные; ns — да).
    let fwd_wh = wh.clone();
    let mut sub = ctl.subscribe_phone();
    let fwd = tokio::spawn(async move {
        use tokio::io::AsyncWriteExt as _;
        loop {
            match sub.recv().await {
                Ok(msg) => {
                    eprintln!("[openmicd] fwd -> {peer:?} seq={} {}", msg.seq, msg.line);
                    let mut w = fwd_wh.lock().await;
                    if w.write_all(msg.line.as_bytes()).await.is_err()
                        || w.write_all(b"\n").await.is_err()
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Дальше C→S — только бинарь. Локальный gain/mute применяются здесь.
    let mut stream = rd.into_inner();
    let mut buf = Vec::with_capacity(4096);
    let mut frames: u64 = 0;
    let mut last_log = Instant::now();
    let mut peak = 0f32;
    loop {
        match read_audio_frame(&mut stream, &mut buf).await {
            Ok(0) => continue,
            Ok(_) => {
                ctl.apply(&mut buf);
                pw.push_bytes(&buf);
                frames += 1;
                ctl.add_frames(1);
                let r = rms_i16le(&buf);
                peak = peak.max(r);
                ctl.push_level(r);
                if last_log.elapsed() > Duration::from_secs(5) {
                    eprintln!("[openmicd] audio: frames={frames} rms_peak={peak:.3} -> pipewire");
                    last_log = Instant::now();
                    peak = 0.0;
                }
            }
            Err(e) => {
                eprintln!("[openmicd] client {peer:?} gone after {frames} frames: {e:#}");
                break;
            }
        }
    }
    fwd.abort();
    ctl.set_connected(false);
    ctl.push_level(0.0);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let tpath = token_path(args.token_file);
    let token = load_or_create_token(&tpath)?;
    eprintln!("[openmicd] token file: {}", tpath.display());
    eprintln!("[openmicd] listening on 0.0.0.0:{}", args.port);

    let pw = pipewire_sink::spawn().context("pipewire spawn")?;
    let ctl = control::ControlState::new();
    tokio::spawn(control::serve(ctl.clone()));

    let listener = TcpListener::bind(("0.0.0.0", args.port)).await?;
    loop {
        let (stream, _) = listener.accept().await?;
        let expected = token.clone();
        let pw = pw.clone();
        let ctl = ctl.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(stream, &expected, pw, ctl).await {
                eprintln!("[openmicd] session error: {e:#}");
            }
        });
    }
}
