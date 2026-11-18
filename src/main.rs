//! Stash: a small, modern package manager front-end for CachyOS / Arch.
//!
//! It never reimplements package logic: every action is a plain `pacman`
//! call (elevated with `pkexec`), so it stays as stable as pacman itself.

use eframe::egui::{self, Color32, RichText};
use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

const ACCENT: Color32 = Color32::from_rgb(47, 128, 237);

/// Hand-picked apps from the official repos, shown on the Discover tab.
const RECOMMENDED: &[(&str, &[(&str, &str)])] = &[
    (
        "Internet & chat",
        &[
            ("firefox", "Fast, private web browser"),
            ("thunderbird", "Email, calendar and contacts"),
            ("telegram-desktop", "Telegram messenger"),
            ("discord", "Voice and text chat"),
            ("signal-desktop", "Private messenger"),
            ("qbittorrent", "Torrent client"),
        ],
    ),
    (
        "Media",
        &[
            ("vlc", "Plays almost any video or audio file"),
            ("mpv", "Minimal, powerful video player"),
            ("obs-studio", "Screen recording and streaming"),
            ("audacity", "Audio recorder and editor"),
            ("kdenlive", "Video editor"),
        ],
    ),
    (
        "Creative",
        &[
            ("gimp", "Photo editing"),
            ("krita", "Digital painting"),
            ("inkscape", "Vector graphics"),
            ("blender", "3D modelling and animation"),
        ],
    ),
    (
        "Office & tools",
        &[
            ("libreoffice-fresh", "Documents, spreadsheets, presentations"),
            ("keepassxc", "Offline password manager"),
            ("btop", "Pretty system monitor for the terminal"),
        ],
    ),
    (
        "Games",
        &[
            ("steam", "Game store and launcher"),
            ("lutris", "Run games from other stores"),
            ("mangohud", "FPS and performance overlay"),
            ("gamemode", "Temporary performance boost while gaming"),
        ],
    ),
];

// ---------------------------------------------------------------- data ----

#[derive(Clone, Debug)]
struct Pkg {
    repo: String,
    name: String,
    version: String,
    desc: String,
    installed: bool,
}

enum Msg {
    Searched(Vec<Pkg>),
    Installed(Vec<Pkg>),
    AllInstalled(HashSet<String>),
    Updates(Vec<Pkg>),
    Log(String),
    Done { ok: bool, label: String },
    Failed(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Discover,
    Installed,
    Updates,
}

#[derive(Clone, Copy)]
enum Kind {
    Search,
    Installed,
    Updates,
}

enum Action {
    Install(Pkg),
    Remove(String),
    UpdateAll,
}

// ------------------------------------------------------------- helpers ----

fn find_in_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '_' | '+' | '-'))
}

fn sv(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

fn capture(prog: &str, args: &[&str]) -> String {
    Command::new(prog)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Finds the user's default terminal: the freedesktop launcher if present,
/// then KDE's setting, then $TERMINAL, then a list of common terminals.
fn default_terminal() -> Option<String> {
    if find_in_path("xdg-terminal-exec") {
        return Some("xdg-terminal-exec".to_string());
    }
    let kde = capture(
        "kreadconfig6",
        &["--file", "kdeglobals", "--group", "General", "--key", "TerminalApplication"],
    );
    let kde = kde.trim();
    if !kde.is_empty() && find_in_path(kde) {
        return Some(kde.to_string());
    }
    if let Ok(t) = std::env::var("TERMINAL") {
        if !t.is_empty() && find_in_path(&t) {
            return Some(t);
        }
    }
    [
        "konsole",
        "ghostty",
        "gnome-terminal",
        "kgx",
        "kitty",
        "alacritty",
        "foot",
        "wezterm",
        "xfce4-terminal",
        "xterm",
    ]
    .iter()
    .find(|t| find_in_path(t))
    .map(|t| t.to_string())
}

/// The flag each terminal needs before the command it should run.
fn terminal_flags(term: &str) -> Vec<&'static str> {
    match term {
        "xdg-terminal-exec" | "kitty" | "foot" => vec![],
        "gnome-terminal" => vec!["--"],
        "xfce4-terminal" => vec!["-x"],
        "wezterm" => vec!["start", "--"],
        _ => vec!["-e"],
    }
}

/// Opens an AUR install (paru/yay) in a terminal. Returns the terminal used.
fn open_in_terminal(helper: &str, pkg: &str) -> Result<String, String> {
    let term = default_terminal().ok_or("No terminal emulator found")?;
    // The package name is passed as an argument, never spliced into the script.
    let script = "\"$0\" -S \"$1\"; echo; printf 'Press Enter to close...'; read _";
    Command::new(&term)
        .args(terminal_flags(&term))
        .args(["sh", "-c", script, helper, pkg])
        .spawn()
        .map(|_| term.clone())
        .map_err(|e| format!("Could not start {term}: {e}"))
}

/// Parses `pacman -Ss` style output:
/// `repo/name version [group] [installed]` followed by an indented description.
fn parse_search(out: &str) -> Vec<Pkg> {
    let mut pkgs = Vec::new();
    let mut lines = out.lines().peekable();
    while let Some(line) = lines.next() {
        if line.starts_with(' ') || line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(id) = parts.next() else { continue };
        let Some((repo, name)) = id.split_once('/') else { continue };
        let version = parts.next().unwrap_or("").to_string();
        let installed = line.to_lowercase().contains("[installed");
        let desc = if lines.peek().map_or(false, |l| l.starts_with(' ')) {
            lines.next().unwrap_or("").trim().to_string()
        } else {
            String::new()
        };
        pkgs.push(Pkg {
            repo: repo.to_string(),
            name: name.to_string(),
            version,
            desc,
            installed,
        });
    }
    pkgs
}

/// Parses `pacman -Qe` output: `name version`.
fn parse_installed(out: &str) -> Vec<Pkg> {
    out.lines()
        .filter_map(|l| {
            let mut p = l.split_whitespace();
            let name = p.next()?;
            let version = p.next().unwrap_or("");
            Some(Pkg {
                repo: String::new(),
                name: name.to_string(),
                version: version.to_string(),
                desc: String::new(),
                installed: true,
            })
        })
        .collect()
}

/// Parses `checkupdates` / `pacman -Qu` output: `name old -> new`.
fn parse_updates(out: &str) -> Vec<Pkg> {
    out.lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.len() < 4 {
                return None;
            }
            Some(Pkg {
                repo: String::new(),
                name: p[0].to_string(),
                version: format!("{} → {}", p[1], p[3]),
                desc: String::new(),
                installed: true,
            })
        })
        .collect()
}

fn spawn_search(tx: Sender<Msg>, ctx: egui::Context, query: String, aur: Option<String>) {
    thread::spawn(move || {
        let mut pkgs = parse_search(&capture("pacman", &["--color", "never", "-Ss", query.as_str()]));
        if let Some(helper) = aur {
            let mut aur_pkgs =
                parse_search(&capture(&helper, &["--color", "never", "-Ssa", query.as_str()]));
            aur_pkgs.retain(|p| p.repo == "aur");
            pkgs.extend(aur_pkgs);
        }
        let _ = tx.send(Msg::Searched(pkgs));
        ctx.request_repaint();
    });
}

fn spawn_installed(tx: Sender<Msg>, ctx: egui::Context) {
    thread::spawn(move || {
        let out = capture("pacman", &["--color", "never", "-Qe"]);
        let _ = tx.send(Msg::Installed(parse_installed(&out)));
        let all = capture("pacman", &["-Qq"]);
        let set: HashSet<String> = all.lines().map(|l| l.trim().to_string()).collect();
        let _ = tx.send(Msg::AllInstalled(set));
        ctx.request_repaint();
    });
}

fn spawn_updates(tx: Sender<Msg>, ctx: egui::Context) {
    thread::spawn(move || {
        // checkupdates (pacman-contrib) syncs a temp database, so it is always fresh.
        let out = if find_in_path("checkupdates") {
            capture("checkupdates", &[])
        } else {
            capture("pacman", &["--color", "never", "-Qu"])
        };
        let _ = tx.send(Msg::Updates(parse_updates(&out)));
        ctx.request_repaint();
    });
}

/// Runs a command in the background and streams its output into the log.
fn run_task(tx: Sender<Msg>, ctx: egui::Context, label: String, prog: String, args: Vec<String>) {
    thread::spawn(move || {
        let _ = tx.send(Msg::Log(format!("\n$ {} {}\n", prog, args.join(" "))));
        let child = Command::new(&prog)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        match child {
            Ok(mut child) => {
                let mut readers = Vec::new();
                if let Some(out) = child.stdout.take() {
                    let (tx, ctx) = (tx.clone(), ctx.clone());
                    readers.push(thread::spawn(move || {
                        for line in BufReader::new(out).lines().map_while(Result::ok) {
                            let _ = tx.send(Msg::Log(line.replace('\r', "\n") + "\n"));
                            ctx.request_repaint();
                        }
                    }));
                }
                if let Some(err) = child.stderr.take() {
                    let (tx, ctx) = (tx.clone(), ctx.clone());
                    readers.push(thread::spawn(move || {
                        for line in BufReader::new(err).lines().map_while(Result::ok) {
                            let _ = tx.send(Msg::Log(line.replace('\r', "\n") + "\n"));
                            ctx.request_repaint();
                        }
                    }));
                }
                for r in readers {
                    let _ = r.join();
                }
                let ok = child.wait().map(|s| s.success()).unwrap_or(false);
                let _ = tx.send(Msg::Done { ok, label });
            }
            Err(e) => {
                let _ = tx.send(Msg::Failed(format!("Could not start {prog}: {e}")));
            }
        }
        ctx.request_repaint();
    });
}

// ----------------------------------------------------------------- app ----

struct Stash {
    ctx: egui::Context,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    tab: Tab,
    query: String,
    filter: String,
    results: Vec<Pkg>,
    installed: Vec<Pkg>,
    all_installed: HashSet<String>,
    updates: Vec<Pkg>,
    searching: bool,
    busy: Option<String>,
    status: String,
    log: String,
    show_log: bool,
    aur: Option<String>,
    confirm_remove: Option<String>,
}

impl Stash {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ctx = cc.egui_ctx.clone();
        ctx.style_mut(|s| {
            s.spacing.item_spacing = egui::vec2(10.0, 10.0);
            s.spacing.button_padding = egui::vec2(14.0, 7.0);
            let r = egui::Rounding::same(8.0);
            s.visuals.widgets.noninteractive.rounding = r;
            s.visuals.widgets.inactive.rounding = r;
            s.visuals.widgets.hovered.rounding = r;
            s.visuals.widgets.active.rounding = r;
            s.visuals.widgets.open.rounding = r;
            s.visuals.selection.bg_fill = ACCENT;
            s.text_styles
                .insert(egui::TextStyle::Heading, egui::FontId::proportional(24.0));
            s.text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
            s.text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(15.0));
        });

        let (tx, rx) = channel();
        let aur = ["paru", "yay"]
            .iter()
            .find(|b| find_in_path(b))
            .map(|b| b.to_string());

        let app = Self {
            ctx,
            tx,
            rx,
            tab: Tab::Discover,
            query: String::new(),
            filter: String::new(),
            results: Vec::new(),
            installed: Vec::new(),
            all_installed: HashSet::new(),
            updates: Vec::new(),
            searching: false,
            busy: None,
            status: "Ready".into(),
            log: String::new(),
            show_log: false,
            aur,
            confirm_remove: None,
        };
        app.refresh_lists();
        app
    }

    fn refresh_lists(&self) {
        spawn_installed(self.tx.clone(), self.ctx.clone());
        spawn_updates(self.tx.clone(), self.ctx.clone());
    }

    fn search(&mut self) {
        let q = self.query.trim().trim_start_matches('-').to_string();
        if q.len() < 2 {
            return;
        }
        self.searching = true;
        spawn_search(self.tx.clone(), self.ctx.clone(), q, self.aur.clone());
    }

    fn start_task(&mut self, label: String, prog: &str, args: Vec<String>) {
        self.busy = Some(label.clone());
        self.show_log = true;
        run_task(self.tx.clone(), self.ctx.clone(), label, prog.to_string(), args);
    }

    fn handle(&mut self, m: Msg) {
        match m {
            Msg::Searched(v) => {
                self.results = v;
                self.searching = false;
            }
            Msg::Installed(v) => self.installed = v,
            Msg::AllInstalled(s) => self.all_installed = s,
            Msg::Updates(v) => self.updates = v,
            Msg::Log(s) => {
                self.log.push_str(&s);
                if self.log.len() > 60_000 {
                    let cut = self.log.len() - 40_000;
                    let cut = (cut..)
                        .find(|i| self.log.is_char_boundary(*i))
                        .unwrap_or(cut);
                    self.log.drain(..cut);
                }
            }
            Msg::Done { ok, label } => {
                self.busy = None;
                self.status = if ok {
                    format!("{label}: done")
                } else if label.starts_with("Installing") {
                    format!("{label}: failed. Package databases may be out of date, try Update all first (see log)")
                } else {
                    format!("{label}: failed (see log)")
                };
                self.refresh_lists();
                if !self.query.trim().is_empty() {
                    self.search();
                }
            }
            Msg::Failed(e) => {
                self.busy = None;
                self.status = e.clone();
                self.log.push_str(&format!("{e}\n"));
            }
        }
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Install(p) => {
                if !valid_name(&p.name) {
                    return;
                }
                if p.repo == "aur" {
                    match self.aur.clone() {
                        Some(helper) => {
                            self.status = match open_in_terminal(&helper, &p.name) {
                                Ok(t) => format!("Opened {t} for the AUR install. Press Refresh when it finishes"),
                                Err(e) => e,
                            };
                        }
                        None => self.status = "Install paru or yay to use AUR packages".into(),
                    }
                } else {
                    let mut args = sv(&["pacman", "-S", "--needed", "--noconfirm"]);
                    args.push(p.name.clone());
                    self.start_task(format!("Installing {}", p.name), "pkexec", args);
                }
            }
            Action::Remove(name) => {
                self.confirm_remove = Some(name);
            }
            Action::UpdateAll => {
                self.start_task(
                    "Updating system".into(),
                    "pkexec",
                    sv(&["pacman", "-Syu", "--noconfirm"]),
                );
            }
        }
    }
}

fn pkg_card(ui: &mut egui::Ui, p: &Pkg, kind: Kind, busy: bool) -> Option<Action> {
    let mut action = None;
    egui::Frame::group(ui.style())
        .rounding(10.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let left = (ui.available_width() - 120.0).max(120.0);
                ui.allocate_ui(egui::vec2(left, 0.0), |ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&p.name).strong().size(16.0));
                            ui.label(RichText::new(&p.version).weak());
                            if p.repo == "aur" {
                                ui.label(RichText::new("AUR").small().color(ACCENT));
                            }
                        });
                        if !p.desc.is_empty() {
                            ui.label(&p.desc);
                        }
                    });
                });
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| match kind {
                        Kind::Search => {
                            if p.installed {
                                ui.add_enabled(false, egui::Button::new("Installed"));
                            } else {
                                let b = egui::Button::new(
                                    RichText::new("Install").color(Color32::WHITE),
                                )
                                .fill(ACCENT);
                                if ui.add_enabled(!busy, b).clicked() {
                                    action = Some(Action::Install(p.clone()));
                                }
                            }
                        }
                        Kind::Installed => {
                            if ui
                                .add_enabled(!busy, egui::Button::new("Remove"))
                                .clicked()
                            {
                                action = Some(Action::Remove(p.name.clone()));
                            }
                        }
                        Kind::Updates => {}
                    },
                );
            });
        });
    action
}

fn pkg_list(ui: &mut egui::Ui, pkgs: &[Pkg], kind: Kind, busy: bool) -> Option<Action> {
    let mut action = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for p in pkgs {
                if let Some(a) = pkg_card(ui, p, kind, busy) {
                    action = Some(a);
                }
            }
        });
    action
}

fn recommended_ui(ui: &mut egui::Ui, all_installed: &HashSet<String>, busy: bool) -> Option<Action> {
    let mut action = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.label(RichText::new("Recommended").strong().size(18.0));
            for (category, items) in RECOMMENDED {
                ui.add_space(6.0);
                ui.label(RichText::new(*category).weak());
                for (name, desc) in items.iter() {
                    let p = Pkg {
                        repo: "extra".to_string(),
                        name: name.to_string(),
                        version: String::new(),
                        desc: desc.to_string(),
                        installed: all_installed.contains(*name),
                    };
                    if let Some(a) = pkg_card(ui, &p, Kind::Search, busy) {
                        action = Some(a);
                    }
                }
            }
        });
    action
}

fn empty_state(ui: &mut egui::Ui, text: &str) {
    ui.add_space(60.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(text).weak().size(16.0));
    });
}

impl eframe::App for Stash {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(m) = self.rx.try_recv() {
            self.handle(m);
        }

        let busy = self.busy.is_some();
        let mut action: Option<Action> = None;
        let mut do_search = false;
        let mut do_refresh = false;

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.heading("Stash");
                ui.add_space(16.0);
                let tabs = [
                    (Tab::Discover, "Discover".to_string()),
                    (Tab::Installed, format!("Installed ({})", self.installed.len())),
                    (Tab::Updates, format!("Updates ({})", self.updates.len())),
                ];
                for (t, label) in tabs {
                    if ui.selectable_label(self.tab == t, label).clicked() {
                        self.tab = t;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Refresh").clicked() {
                        do_refresh = true;
                    }
                });
            });
            ui.add_space(8.0);
        });

        egui::TopBottomPanel::bottom("bottom").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if let Some(b) = &self.busy {
                    ui.spinner();
                    ui.label(format!("{b}…"));
                } else {
                    ui.label(RichText::new(&self.status).weak());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.toggle_value(&mut self.show_log, "Log");
                });
            });
            if self.show_log {
                egui::ScrollArea::vertical()
                    .max_height(170.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.label(RichText::new(&self.log).monospace().small());
                    });
            }
            ui.add_space(4.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Discover => {
                ui.horizontal(|ui| {
                    let w = (ui.available_width() - 100.0).max(100.0);
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text("Search apps, tools, libraries…")
                            .desired_width(w),
                    );
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        do_search = true;
                    }
                    if ui.button("Search").clicked() {
                        do_search = true;
                    }
                });
                if self.searching {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Searching…");
                    });
                } else if self.query.trim().is_empty() {
                    if let Some(a) = recommended_ui(ui, &self.all_installed, busy) {
                        action = Some(a);
                    }
                } else if self.results.is_empty() {
                    empty_state(ui, "No results");
                } else if let Some(a) = pkg_list(ui, &self.results, Kind::Search, busy) {
                    action = Some(a);
                }
            }
            Tab::Installed => {
                ui.add(
                    egui::TextEdit::singleline(&mut self.filter)
                        .hint_text("Filter installed packages…")
                        .desired_width(f32::INFINITY),
                );
                let needle = self.filter.to_lowercase();
                let shown: Vec<Pkg> = self
                    .installed
                    .iter()
                    .filter(|p| p.name.to_lowercase().contains(&needle))
                    .cloned()
                    .collect();
                if shown.is_empty() {
                    empty_state(ui, "Nothing here");
                } else if let Some(a) = pkg_list(ui, &shown, Kind::Installed, busy) {
                    action = Some(a);
                }
            }
            Tab::Updates => {
                let n = self.updates.len();
                ui.horizontal(|ui| {
                    ui.label(if n == 0 {
                        "Everything is up to date".to_string()
                    } else {
                        format!("{n} updates available")
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if n > 0 {
                            let b = egui::Button::new(
                                RichText::new("Update all").color(Color32::WHITE),
                            )
                            .fill(ACCENT);
                            if ui.add_enabled(!busy, b).clicked() {
                                action = Some(Action::UpdateAll);
                            }
                        }
                    });
                });
                if n > 0 {
                    pkg_list(ui, &self.updates, Kind::Updates, busy);
                }
            }
        });

        // Remove confirmation dialog.
        if let Some(name) = self.confirm_remove.clone() {
            let mut cancel = false;
            let mut go = false;
            egui::Window::new("Remove package?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!("Remove {name} and the dependencies nothing else needs?"));
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                        if ui.button("Remove").clicked() {
                            go = true;
                        }
                    });
                });
            if cancel {
                self.confirm_remove = None;
            }
            if go {
                self.confirm_remove = None;
                if valid_name(&name) {
                    let mut args = sv(&["pacman", "-Rs", "--noconfirm"]);
                    args.push(name.clone());
                    self.start_task(format!("Removing {name}"), "pkexec", args);
                }
            }
        }

        if let Some(a) = action {
            self.apply(a);
        }
        if do_search {
            self.search();
        }
        if do_refresh {
            self.refresh_lists();
            if !self.query.trim().is_empty() {
                self.search();
            }
            self.status = "Refreshed".into();
        }
    }
}

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Stash")
            .with_app_id("stash")
            .with_inner_size([940.0, 680.0])
            .with_min_inner_size([640.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Stash",
        options,
        Box::new(|cc| Ok(Box::new(Stash::new(cc)))),
    )
}
