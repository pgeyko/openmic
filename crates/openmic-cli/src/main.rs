//! openmic — CLI к демону: status / doctor / тестовый tonesend.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use openmic_proto::{write_audio_frame, write_json_line, Hello, DEFAULT_PORT};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;

#[derive(Parser, Debug)]
#[command(name = "openmic", about = "OpenMic client")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Проверить окружение: PipeWire, сеть, токен
    Doctor,
    /// Показать токен/QR-данные для сопряжения + состояние демона
    Status,
    /// Усиление 0–200% (применяется демоном)
    Gain {
        value: u16,
    },
    /// Заглушить/разглушить
    Mute,
    Unmute,
    /// Шумодав на телефоне: on | off
    Ns {
        state: String,
    },
    /// Тест: handshake + 2 с тишины + 2 с тона 440 Гц (проверка демона без телефона)
    Tonesend {
        #[arg(default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
}

fn sock_path() -> PathBuf {
    openmic_proto::local::control_sock_path()
}

async fn ctl_send(req: &str) -> Result<String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;
    let mut s = UnixStream::connect(sock_path()).await?;
    s.write_all(req.as_bytes()).await?;
    s.write_all(b"\n").await?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).await?;
    Ok(line.trim().to_string())
}

fn token_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let mut h = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()));
            h.push(".config");
            h
        });
    base.join("openmic/token")
}

fn doctor() -> Result<()> {
    let mut ok = true;
    let mut check = |name: &str, pass: bool, hint: &str| {
        println!("{} {name}{}", if pass { "OK  " } else { "FAIL" }, if pass { String::new() } else { format!(" — {hint}") });
        ok &= pass;
    };
    check(
        "pw-cli",
        std::process::Command::new("pw-cli").arg("--version").output().is_ok(),
        "нет pipewire-utils",
    );
    check(
        "pipewire running",
        std::process::Command::new("pactl")
            .arg("info")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false),
        "pactl info неуспешен — запущен ли PipeWire?",
    );
    let t = token_path();
    check("token file", t.exists(), &format!("нет {} — запустите openmicd", t.display()));
    if !ok {
        anyhow::bail!("doctor: есть проблемы (см. FAIL выше)");
    }
    println!("doctor: всё хорошо");
    Ok(())
}

async fn status() -> Result<()> {
    let t = token_path();
    let token = std::fs::read_to_string(&t)
        .with_context(|| format!("нет токена: {}", t.display()))?;
    let token = token.trim();
    let ip = local_ip_hint();
    println!("token: {token}");
    println!("qr-json: {{\"ip\":\"{ip}\",\"port\":{DEFAULT_PORT},\"token\":\"{token}\"}}");
    let cands = lan_candidates();
    if cands.len() > 1 {
        println!("lan-ips: {} (если телефон в другой сети — подставьте нужный)", cands.join(", "));
    }
    println!("(QR-картинка — следующим шагом; пока вбейте JSON в телефон вручную)");
    // Состояние демона — best-effort (демон может не быть запущен).
    match ctl_send("{\"status\":true}").await {
        Ok(line) => println!("daemon: {line}"),
        Err(e) => println!("daemon: unreachable ({e})"),
    }
    Ok(())
}

/// Все глобальные IPv4, сначала приватные LAN — см. openmic_proto::local.
fn lan_candidates() -> Vec<String> {
    openmic_proto::local::lan_candidates()
}

fn local_ip_hint() -> String {
    openmic_proto::local::lan_ip_hint()
}

async fn tonesend(host: &str, port: u16) -> Result<()> {
    let token = std::fs::read_to_string(token_path())?.trim().to_string();
    let stream = TcpStream::connect((host, port)).await?;
    let (rh, mut wh) = stream.into_split();
    let mut rd = BufReader::new(rh);
    write_json_line(&mut wh, &Hello::new(&token)).await?;
    let mut line = String::new();
    rd.read_line(&mut line).await?;
    println!("welcome: {}", line.trim());
    if !line.contains("\"ok\":true") {
        anyhow::bail!("handshake rejected");
    }
    // 2 с тишины (50 фреймов по 20 мс), затем 2 с тона 440 Гц.
    let frame_samples = 960usize; // 20 мс при 48 кГц
    let silence = vec![0u8; frame_samples * 2];
    for _ in 0..100 {
        write_audio_frame(&mut wh, &silence).await?;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    for i in 0..100 {
        let mut pcm = Vec::with_capacity(frame_samples * 2);
        for n in 0..frame_samples {
            let t = (i * frame_samples + n) as f32 / 48000.0;
            let s = (f32::sin(t * 440.0 * 2.0 * std::f32::consts::PI) * 20000.0) as i16;
            pcm.extend_from_slice(&s.to_le_bytes());
        }
        write_audio_frame(&mut wh, &pcm).await?;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    println!("tonesend: done (смотри лог openmicd: frames/rms)");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Doctor => doctor(),
        Cmd::Status => status().await,
        Cmd::Gain { value } => {
            let line = ctl_send(&format!("{{\"gain\":{value}}}")).await?;
            println!("{line}");
            Ok(())
        }
        Cmd::Mute => {
            let line = ctl_send("{\"mute\":true}").await?;
            println!("{line}");
            Ok(())
        }
        Cmd::Unmute => {
            let line = ctl_send("{\"mute\":false}").await?;
            println!("{line}");
            Ok(())
        }
        Cmd::Ns { state } => {
            let v = match state.as_str() {
                "on" | "true" | "1" => true,
                "off" | "false" | "0" => false,
                _ => anyhow::bail!("usage: openmic ns on|off"),
            };
            let line = ctl_send(&format!("{{\"ns\":{v}}}")).await?;
            println!("{line}");
            Ok(())
        }
        Cmd::Tonesend { host, port } => tonesend(&host, port).await,
    }
}
