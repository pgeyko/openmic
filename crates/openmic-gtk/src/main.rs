//! OpenMic GTK4 GUI: состояние, визуализатор звука, gain/mute/шумодав, сопряжение.
//!
//! Данные тянет из демона через control-сокет (`{"watch":true}` → 10 Гц).
//! Без демона показывает «не запущен», но QR и поля рисует всё равно.

use adw::prelude::*;
use gtk::{gdk, gdk_pixbuf, glib};
use serde::Deserialize;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default, Deserialize)]
#[allow(dead_code)]
struct State {
    #[serde(default)]
    level: f32,
    #[serde(default)]
    gain: u16,
    #[serde(default)]
    mute: bool,
    #[serde(default)]
    connected: bool,
    #[serde(default)]
    frames: u64,
    phone_ns: Option<bool>,
    #[serde(default)]
    ok: bool,
}

/// Разовый запрос к демону. None — демон не отвечает.
fn send(req: &str) -> Option<State> {
    let path = openmic_proto::local::control_sock_path();
    let mut s = UnixStream::connect(path).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    s.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
    writeln!(s, "{req}").ok()?;
    let line = BufReader::new(s).lines().next()?.ok()?;
    serde_json::from_str(&line).ok()
}

fn qr_pixbuf(data: &str, scale: usize) -> Option<gdk_pixbuf::Pixbuf> {
    use qrcode::QrCode;
    let code = QrCode::new(data.as_bytes()).ok()?;
    let w = code.width();
    let quiet = 2usize;
    let size = (w + quiet * 2) * scale;
    let mut px = vec![255u8; size * size * 3];
    for y in 0..w {
        for x in 0..w {
            if code[(x, y)] == qrcode::Color::Dark {
                for dy in 0..scale {
                    for dx in 0..scale {
                        let ox = (x + quiet) * scale + dx;
                        let oy = (y + quiet) * scale + dy;
                        let i = (oy * size + ox) * 3;
                        px[i] = 0;
                        px[i + 1] = 0;
                        px[i + 2] = 0;
                    }
                }
            }
        }
    }
    let bytes = glib::Bytes::from(&px);
    Some(gdk_pixbuf::Pixbuf::from_bytes(
        &bytes,
        gdk_pixbuf::Colorspace::Rgb,
        false,
        8,
        size as i32,
        size as i32,
        (size * 3) as i32,
    ))
}

fn copy_button(text: &str) -> gtk::Button {
    let b = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Скопировать")
        .valign(gtk::Align::Center)
        .build();
    let text = text.to_string();
    b.connect_clicked(move |_| {
        if let Some(d) = gdk::Display::default() {
            d.clipboard().set_text(&text);
        }
    });
    b
}

/// Строка «Имя: значение [копировать]» для параметров сопряжения.
fn field_row(name: &str, value: &str) -> (gtk::Box, gtk::Label) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name_lbl = gtk::Label::new(Some(name));
    name_lbl.set_width_chars(6);
    name_lbl.set_halign(gtk::Align::Start);
    name_lbl.add_css_class("dim-label");

    let val = gtk::Label::builder()
        .label(value)
        .selectable(true)
        .halign(gtk::Align::Start)
        .hexpand(true)
        .build();
    val.add_css_class("monospace");

    row.append(&name_lbl);
    row.append(&val);
    row.append(&copy_button(value));
    (row, val)
}

fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

struct DaemonGuard {
    child: Option<Child>,
}

fn find_daemon() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let c = dir.join("openmicd");
            if c.is_file() {
                return c;
            }
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let c = PathBuf::from(&home).join(".local/bin/openmicd");
        if c.is_file() {
            return c;
        }
    }
    let c = PathBuf::from("target/debug/openmicd");
    if c.is_file() {
        return c;
    }
    PathBuf::from("openmicd")
}

impl DaemonGuard {
    fn start_if_needed(&mut self) -> bool {
        // Поддержка USB-режима: пробрасываем порт с телефона на ПК (reverse: телефон 127.0.0.1:59100 -> ПК 59100).
        // Внимание: adb forward слушает порт на ПК и блокирует запуск openmicd. Нужен adb reverse!
        std::thread::spawn(|| {
            let _ = Command::new("adb")
                .args(["reverse", &format!("tcp:{}", openmic_proto::DEFAULT_PORT), &format!("tcp:{}", openmic_proto::DEFAULT_PORT)])
                .output();
        });

        if send("{\"status\":true}").is_some() {
            eprintln!("[gui] daemon already running, attaching");
            return true;
        }
        let bin = find_daemon();
        let mut cmd = Command::new(&bin);
        cmd.arg("--port")
            .arg(openmic_proto::DEFAULT_PORT.to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        use std::os::unix::process::CommandExt as _;
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM as libc::c_ulong);
                Ok(())
            });
        }
        let child = cmd.spawn();
        match child {
            Ok(child) => {
                eprintln!("[gui] spawned daemon {}", bin.display());
                self.child = Some(child);
                for _ in 0..50 {
                    std::thread::sleep(Duration::from_millis(100));
                    if send("{\"status\":true}").is_some() {
                        return true;
                    }
                }
                eprintln!("[gui] daemon did not answer in 5s");
                false
            }
            Err(e) => {
                eprintln!("[gui] cannot spawn {}: {e}", bin.display());
                false
            }
        }
    }

    fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            eprintln!("[gui] stopping own daemon");
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn build_ui(app: &adw::Application, guard: Rc<RefCell<DaemonGuard>>) {
    guard.borrow_mut().start_if_needed();

    let token =
        openmic_proto::local::read_token().unwrap_or_else(|| "нет токена — запустите openmicd".into());
    let ip = openmic_proto::local::lan_ip_hint();
    let port = openmic_proto::DEFAULT_PORT.to_string();
    let qr_json = format!("{{\"ip\":\"{ip}\",\"port\":{port},\"token\":\"{token}\"}}");

    // --- Карточка 1: Статус эфира и Живая Волна ---
    let dot = gtk::Label::new(Some("●"));
    let daemon_state = gtk::Label::builder().halign(gtk::Align::Start).hexpand(true).build();
    let phone_state = gtk::Label::builder().halign(gtk::Align::Start).hexpand(true).build();
    let start_btn = gtk::Button::with_label("Запустить демона");
    start_btn.set_valign(gtk::Align::Center);
    {
        let guard = guard.clone();
        start_btn.connect_clicked(move |_| {
            guard.borrow_mut().start_if_needed();
        });
    }

    let status_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    status_row.append(&dot);
    status_row.append(&daemon_state);
    status_row.append(&start_btn);

    // История уровней для визуализатора (до 48 столбиков)
    let wave_history = Arc::new(Mutex::new(VecDeque::<f32>::with_capacity(48)));

    // Кастомный виджет визуализатора звука на DrawingArea
    let wave_area = gtk::DrawingArea::builder()
        .content_width(360)
        .content_height(64)
        .build();

    {
        let hist = wave_history.clone();
        wave_area.set_draw_func(move |_, cr, width, height| {
            let w = width as f64;
            let h = height as f64;

            // Тёмный фон визуализатора со скруглёнными углами
            cr.set_source_rgb(0.08, 0.08, 0.08);
            let r = 8.0;
            cr.new_sub_path();
            cr.arc(w - r, r, r, -std::f64::consts::FRAC_PI_2, 0.0);
            cr.arc(w - r, h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
            cr.arc(r, h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
            cr.arc(r, r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
            cr.close_path();
            cr.fill().ok();

            let q = hist.lock().unwrap();
            if q.is_empty() {
                // Линия тишины
                cr.set_source_rgb(0.2, 0.2, 0.2);
                cr.set_line_width(1.5);
                cr.move_to(12.0, h / 2.0);
                cr.line_to(w - 12.0, h / 2.0);
                cr.stroke().ok();
                return;
            }

            let cap = 48;
            let step = (w - 24.0) / cap as f64;
            let bar_w = (step * 0.65).max(3.0);
            let n = q.len();

            for (i, &v) in q.iter().enumerate() {
                let x = 12.0 + (cap - n + i) as f64 * step + (step - bar_w) / 2.0;
                let bar_h = (v as f64 * (h * 0.85)).max(4.0);
                let top = (h - bar_h) / 2.0;

                if v >= 0.95 {
                    // Клиппинг - красный
                    cr.set_source_rgb(1.0, 0.32, 0.32);
                } else {
                    // Сине-зелёный неоновый цвет звука
                    cr.set_source_rgb(0.16, 0.65, 0.95);
                }

                // Скруглённые столбики
                let br = bar_w / 2.0;
                cr.new_sub_path();
                cr.arc(x + br, top + br, br, std::f64::consts::PI, 0.0);
                cr.arc(x + br, top + bar_h - br, br, 0.0, std::f64::consts::PI);
                cr.close_path();
                cr.fill().ok();
            }
        });
    }

    let card_status = adw::PreferencesGroup::new();
    card_status.set_title("Состояние");
    let status_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    status_box.append(&status_row);
    status_box.append(&phone_state);
    status_box.append(&wave_area);
    card_status.add(&status_box);

    // --- Карточка 2: Настройки звука ---
    let card_audio = adw::PreferencesGroup::new();
    card_audio.set_title("Параметры звука");

    let gain_row = adw::ActionRow::builder()
        .title("Усиление")
        .subtitle("Чувствительность входного потока")
        .build();
    let gain_val = gtk::Label::new(Some("100%"));
    gain_val.set_width_chars(5);
    gain_val.add_css_class("monospace");
    let gain = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 200.0, 1.0);
    gain.set_hexpand(true);
    gain.set_draw_value(false);
    gain.set_valign(gtk::Align::Center);
    let scale_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    scale_box.append(&gain);
    scale_box.append(&gain_val);
    gain_row.add_suffix(&scale_box);

    let mic_row = adw::ActionRow::builder()
        .title("Микрофон включён")
        .subtitle("Заглушить отдачу звука в систему")
        .build();
    let mute = gtk::Switch::new();
    mute.set_active(true);
    mute.set_valign(gtk::Align::Center);
    mic_row.add_suffix(&mute);

    let ns_row = adw::ActionRow::builder()
        .title("Шумоподавление")
        .subtitle("Аппаратный шумодав на смартфоне")
        .build();
    let ns = gtk::Switch::new();
    ns.set_valign(gtk::Align::Center);
    ns_row.add_suffix(&ns);

    card_audio.add(&gain_row);
    card_audio.add(&mic_row);
    card_audio.add(&ns_row);

    // Начальное состояние
    if let Some(st) = send("{\"status\":true}") {
        gain.set_value(st.gain as f64);
        gain_val.set_label(&format!("{}%", st.gain));
        mute.set_active(!st.mute);
        if let Some(v) = st.phone_ns {
            ns.set_active(v);
        }
    }

    gain.connect_value_changed({
        let gain_val = gain_val.clone();
        move |s| {
            let v = s.value() as u16;
            gain_val.set_label(&format!("{v}%"));
            send(&format!("{{\"gain\":{v}}}"));
        }
    });

    mute.connect_state_set(|_, on| {
        send(&format!("{{\"mute\":{}}}", !on));
        glib::Propagation::Proceed
    });

    ns.connect_state_set(|_, on| {
        send(&format!("{{\"ns\":{on}}}"));
        glib::Propagation::Proceed
    });

    // --- Карточка 3: Сопряжение ---
    let card_conn = adw::PreferencesGroup::new();
    card_conn.set_title("Сопряжение телефона");

    let conn_box = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    let pic = gtk::Picture::new();
    pic.set_size_request(130, 130);
    pic.set_valign(gtk::Align::Start);
    if let Some(pb) = qr_pixbuf(&qr_json, 3) {
        pic.set_pixbuf(Some(&pb));
    }
    conn_box.append(&pic);

    let fields = gtk::Box::new(gtk::Orientation::Vertical, 4);
    fields.set_hexpand(true);
    fields.set_valign(gtk::Align::Center);
    let (ip_row, ip_val_lbl) = field_row("IP", &ip);
    let (port_row, _) = field_row("Порт", &port);
    fields.append(&ip_row);
    fields.append(&port_row);

    // Токен
    let tok_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let tok_name = gtk::Label::new(Some("Токен"));
    tok_name.set_width_chars(6);
    tok_name.set_halign(gtk::Align::Start);
    tok_name.add_css_class("dim-label");
    let tok_val = gtk::Label::builder()
        .label(&token)
        .selectable(true)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .halign(gtk::Align::Start)
        .hexpand(true)
        .build();
    tok_val.add_css_class("monospace");
    tok_row.append(&tok_name);
    tok_row.append(&tok_val);
    tok_row.append(&copy_button(&token));
    fields.append(&tok_row);

    conn_box.append(&fields);
    card_conn.add(&conn_box);

    // Главный контейнер Adwaita
    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(16);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.append(&card_status);
    content.append(&card_audio);
    content.append(&card_conn);

    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&content)
        .build();

    // --- Поток данных 10 Гц ---
    let shared = Arc::new(Mutex::new((State::default(), false)));
    let shared_w = shared.clone();
    std::thread::spawn(move || {
        let path = openmic_proto::local::control_sock_path();
        loop {
            let mut s = match UnixStream::connect(&path) {
                Ok(s) => s,
                Err(_) => {
                    if let Ok(mut g) = shared_w.lock() {
                        *g = (State::default(), false);
                    }
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };
            if writeln!(s, "{{\"watch\":true}}").is_err() {
                continue;
            }
            s.set_read_timeout(Some(Duration::from_secs(5))).ok();
            let mut alive = false;
            for line in BufReader::new(s).lines().map_while(Result::ok) {
                if let Ok(st) = serde_json::from_str::<State>(&line) {
                    alive = true;
                    if let Ok(mut g) = shared_w.lock() {
                        *g = (st, true);
                    }
                }
            }
            if alive {
                if let Ok(mut g) = shared_w.lock() {
                    *g = (State::default(), false);
                }
            }
        }
    });

    let wave_area_w = wave_area.clone();
    let hist_w = wave_history.clone();
    let mut air_since: Option<Instant> = None;
    let mut was_connected = false;
    let mut fast_ticks: u32 = 0;

    let mut last_ip = ip.clone();
    let pic_c = pic.clone();
    let ip_val_c = ip_val_lbl.clone();
    let token_c = token.clone();
    let port_c = port.clone();

    glib::timeout_add_local(Duration::from_millis(100), move || {
        let (st, up) = shared
            .lock()
            .map(|g| g.clone())
            .unwrap_or((State::default(), false));

        // Обновляем буфер волны
        {
            let mut q = hist_w.lock().unwrap();
            if up && st.connected && !st.mute {
                if q.len() >= 48 {
                    q.pop_front();
                }
                q.push_back(st.level);
            } else {
                q.clear();
            }
        }
        wave_area_w.queue_draw();

        if up && mute.is_active() == st.mute {
            mute.set_active(!st.mute);
        }

        start_btn.set_visible(!up);
        if !up {
            daemon_state.set_label("Демон: остановлен");
            phone_state.set_label("");
            dot.set_label("●");
            dot.add_css_class("dim-label");
            dot.remove_css_class("success");
            air_since = None;
            was_connected = false;
        } else {
            daemon_state.set_label("Демон: активен");
            if st.connected {
                if !was_connected {
                    air_since = Some(Instant::now());
                    was_connected = true;
                }
                let dur = air_since
                    .map(|t| fmt_dur(t.elapsed()))
                    .unwrap_or_else(|| "0:00".into());
                if st.mute {
                    phone_state.set_label(&format!("Смартфон: в эфире {dur} (заглушён)"));
                } else {
                    phone_state.set_label(&format!("Смартфон: в эфире {dur}"));
                }
                dot.set_label("●");
                dot.add_css_class("success");
                dot.remove_css_class("dim-label");
            } else {
                phone_state.set_label("Смартфон: ожидает подключения…");
                dot.set_label("●");
                dot.add_css_class("dim-label");
                dot.remove_css_class("success");
                air_since = None;
                was_connected = false;
            }
        }
        // Периодическая проверка смены IP адреса ПК (раз в 3 секунды)
        if fast_ticks % 30 == 0 {
            let current_ip = openmic_proto::local::lan_ip_hint();
            if current_ip != last_ip {
                last_ip = current_ip.clone();
                ip_val_c.set_label(&current_ip);
                let new_qr = format!("{{\"ip\":\"{current_ip}\",\"port\":{port_c},\"token\":\"{token_c}\"}}");
                if let Some(pb) = qr_pixbuf(&new_qr, 3) {
                    pic_c.set_pixbuf(Some(&pb));
                }
            }
        }
        fast_ticks = fast_ticks.wrapping_add(1);

        glib::ControlFlow::Continue
    });

    let win = adw::ApplicationWindow::builder()
        .application(app)
        .title("OpenMic")
        .default_width(460)
        .default_height(540)
        .content(&scroll)
        .build();
    win.present();
}

fn main() {
    let guard: Rc<RefCell<DaemonGuard>> = Rc::new(RefCell::new(DaemonGuard { child: None }));
    let app = adw::Application::builder()
        .application_id("com.openmic.Gui")
        .build();
    {
        let guard = guard.clone();
        app.connect_activate(move |app| build_ui(app, guard.clone()));
    }
    {
        let guard = guard.clone();
        app.connect_shutdown(move |_| guard.borrow_mut().stop());
    }
    app.run();
}
