//! Локальные хелперы ПК-стороны: путь токена, LAN-адреса, control-сокет.
//! Без новых зависимостей (только std) — используют демон, CLI и GUI.

use std::path::PathBuf;

/// `~/.config/openmic/token` (или `$XDG_CONFIG_HOME/openmic/token`).
pub fn token_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let mut h = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()));
            h.push(".config");
            h
        });
    base.join("openmic/token")
}

pub fn read_token() -> Option<String> {
    std::fs::read_to_string(token_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Все глобальные IPv4, сначала приватные LAN (192.168/10/172.16-31),
/// затем остальные (туннели типа 100.64/10 — в конце).
pub fn lan_candidates() -> Vec<String> {
    let out = std::process::Command::new("ip")
        .args(["-o", "-4", "addr", "show", "scope", "global"])
        .output();
    let mut ips: Vec<String> = Vec::new();
    if let Ok(o) = out {
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            let mut it = line.split_whitespace();
            while let Some(w) = it.next() {
                if w == "inet" {
                    if let Some(cidr) = it.next() {
                        if let Some(ip) = cidr.split('/').next() {
                            if !ip.starts_with("127.") && !ips.contains(&ip.to_string()) {
                                ips.push(ip.to_string());
                            }
                        }
                    }
                    break;
                }
            }
        }
    }
    ips.sort_by_key(|a| {
        if a.starts_with("192.168.") || a.starts_with("10.") {
            0
        } else if a.starts_with("172.") {
            1
        } else {
            2
        }
    });
    ips
}

pub fn lan_ip_hint() -> String {
    lan_candidates()
        .into_iter()
        .next()
        .unwrap_or_else(|| "127.0.0.1".into())
}

/// `$XDG_RUNTIME_DIR/openmic.sock` (фолбэк `/tmp/openmic-$USER.sock`).
pub fn control_sock_path() -> PathBuf {
    if let Ok(rt) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(rt).join("openmic.sock");
    }
    let user = std::env::var("USER").unwrap_or_else(|_| "def".into());
    PathBuf::from(format!("/tmp/openmic-{user}.sock"))
}
