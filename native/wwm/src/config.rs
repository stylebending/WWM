//! Configuration: `wwm.toml` beside the repo root (or defaults when absent).

use std::collections::HashMap;

use log::debug;
use serde::Deserialize;

use crate::commands::Direction;
use crate::vda::find_file_upward;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mods {
    pub win: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

#[derive(Clone, Debug)]
pub enum Action {
    Switch(i32),
    Move(i32),
    Focus(Direction),
    Reorder(Direction),
    Snap(Direction),
    Float,
    Overlay,
    Close,
    Maximize,
    Quit,
    Run(String),
}

#[derive(Clone, Debug)]
pub struct Keybind {
    pub mods: Mods,
    pub vk: u32,
    pub action: Action,
}

pub struct Config {
    pub master_ratio: f64,
    pub tile_interval_ms: u64,
    pub min_tile_width: i32,
    pub min_tile_height: i32,
    pub gap: i32,
    pub default_float: bool,
    pub overlay_ms: u64,
    pub float_process_names: Vec<String>,
    pub float_class_names: Vec<String>,
    pub keybinds: Vec<Keybind>,
}

impl Config {
    pub fn load() -> Self {
        let mut cfg = Self::defaults();
        if let Some(path) = find_file_upward("wwm.toml") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                match toml::from_str::<RawConfig>(&text) {
                    Ok(raw) => {
                        cfg.apply(raw);
                        debug!("loaded config {} ({} keybinds)", path.display(), cfg.keybinds.len());
                    }
                    Err(e) => {
                        debug!("config {} parse error: {e}", path.display());
                    }
                }
            }
        }
        cfg
    }

    fn defaults() -> Self {
        let mut keybinds = Vec::new();

        for (key, n) in [
            ('1', 0), ('2', 1), ('3', 2), ('4', 3), ('5', 4),
            ('6', 5), ('7', 6), ('8', 7), ('9', 8), ('0', 9),
        ] {
            keybinds.push(Keybind {
                mods: Mods { win: true, shift: false, ctrl: false, alt: false },
                vk: key as u32,
                action: Action::Switch(n),
            });
            keybinds.push(Keybind {
                mods: Mods { win: true, shift: true, ctrl: false, alt: false },
                vk: key as u32,
                action: Action::Move(n),
            });
        }

        for (key, dir) in [
            ('h', Direction::Left), ('j', Direction::Down),
            ('k', Direction::Up), ('l', Direction::Right),
        ] {
            let vk = key.to_ascii_uppercase() as u32;
            keybinds.push(Keybind { mods: Mods { win: true, shift: false, ctrl: false, alt: false }, vk, action: Action::Focus(dir) });
            keybinds.push(Keybind { mods: Mods { win: true, shift: true, ctrl: false, alt: false }, vk, action: Action::Reorder(dir) });
            keybinds.push(Keybind { mods: Mods { win: true, shift: false, ctrl: true, alt: false }, vk, action: Action::Snap(dir) });
        }

        keybinds.push(Keybind { mods: Mods { win: true, shift: true, ctrl: false, alt: false }, vk: 0x20, action: Action::Float });
        keybinds.push(Keybind { mods: Mods { win: true, shift: false, ctrl: false, alt: false }, vk: 0x20, action: Action::Overlay });
        keybinds.push(Keybind { mods: Mods { win: true, shift: false, ctrl: false, alt: false }, vk: 'Q' as u32, action: Action::Close });
        keybinds.push(Keybind { mods: Mods { win: true, shift: false, ctrl: false, alt: false }, vk: 'F' as u32, action: Action::Maximize });
        keybinds.push(Keybind { mods: Mods { win: true, shift: false, ctrl: true, alt: false }, vk: 0x1B, action: Action::Quit });

        keybinds.push(Keybind { mods: Mods { win: true, shift: false, ctrl: false, alt: false }, vk: 'B' as u32, action: Action::Run("brave.exe".into()) });
        keybinds.push(Keybind { mods: Mods { win: true, shift: false, ctrl: false, alt: false }, vk: 'E' as u32, action: Action::Run("explorer.exe".into()) });
        keybinds.push(Keybind { mods: Mods { win: true, shift: false, ctrl: false, alt: false }, vk: 'T' as u32, action: Action::Run("wt.exe".into()) });
        keybinds.push(Keybind { mods: Mods { win: true, shift: false, ctrl: false, alt: false }, vk: 'N' as u32, action: Action::Run("notepad.exe".into()) });

        Self {
            master_ratio: 0.5,
            tile_interval_ms: 250,
            min_tile_width: 160,
            min_tile_height: 120,
            gap: 0,
            default_float: false,
            overlay_ms: 1100,
            float_process_names: vec![
                "applicationframehost.exe".into(),
                "shellexperiencehost.exe".into(),
                "searchhost.exe".into(),
                "startmenuexperiencehost.exe".into(),
                "textinputhost.exe".into(),
            ],
            float_class_names: vec![
                "shell_traywnd".into(),
                "shell_secondarytraywnd".into(),
                "progman".into(),
                "workerw".into(),
                "windows.ui.core.corewindow".into(),
                "xamlexplorerhostislandwindow".into(),
                "multitaskingviewframe".into(),
                "foregroundstaging".into(),
            ],
            keybinds,
        }
    }

    fn apply(&mut self, raw: RawConfig) {
        if let Some(t) = raw.tiling {
            if let Some(v) = t.master_ratio { self.master_ratio = v; }
            if let Some(v) = t.tile_interval_ms { self.tile_interval_ms = v; }
            if let Some(v) = t.min_tile_width { self.min_tile_width = v; }
            if let Some(v) = t.min_tile_height { self.min_tile_height = v; }
            if let Some(v) = t.gap { self.gap = v.max(0); }
            if let Some(v) = t.default_mode {
                self.default_float = match v.to_ascii_lowercase().as_str() {
                    "float" | "floating" => true,
                    "tile" | "tiled" | "tiling" => false,
                    _ => {
                        debug!("ignoring unknown tiling.default_mode {v}");
                        self.default_float
                    }
                };
            }
        }
        if let Some(f) = raw.float {
            if let Some(v) = f.process_names { self.float_process_names = v; }
            if let Some(v) = f.class_names { self.float_class_names = v; }
        }
        if let Some(o) = raw.overlay {
            if let Some(v) = o.duration_ms { self.overlay_ms = v; }
        }
        if let Some(map) = raw.keybinds {
            for (spec, action) in map {
                if let (Some((mods, vk)), Some(action)) = (parse_mods_key(&spec), parse_action(&action)) {
                    self.keybinds.retain(|b| !(b.mods == mods && b.vk == vk));
                    self.keybinds.push(Keybind { mods, vk, action });
                }
            }
        }
    }
}

#[derive(Deserialize, Default)]
struct RawConfig {
    tiling: Option<RawTiling>,
    float: Option<RawFloat>,
    overlay: Option<RawOverlay>,
    keybinds: Option<HashMap<String, String>>,
}

#[derive(Deserialize)]
struct RawTiling {
    master_ratio: Option<f64>,
    tile_interval_ms: Option<u64>,
    min_tile_width: Option<i32>,
    min_tile_height: Option<i32>,
    gap: Option<i32>,
    default_mode: Option<String>,
}

#[derive(Deserialize)]
struct RawFloat {
    process_names: Option<Vec<String>>,
    class_names: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct RawOverlay {
    duration_ms: Option<u64>,
}

fn parse_mods_key(spec: &str) -> Option<(Mods, u32)> {
    let mut parts: Vec<&str> = spec.split('+').map(|s| s.trim()).collect();
    let key = parts.pop()?;
    let mut mods = Mods { win: false, shift: false, ctrl: false, alt: false };
    for p in parts {
        match p.to_ascii_lowercase().as_str() {
            "win" => mods.win = true,
            "shift" => mods.shift = true,
            "ctrl" | "control" => mods.ctrl = true,
            "alt" => mods.alt = true,
            _ => return None,
        }
    }
    Some((mods, parse_key(key)?))
}

fn parse_key(key: &str) -> Option<u32> {
    let k = key.to_ascii_lowercase();
    match k.as_str() {
        "space" => Some(0x20),
        "escape" | "esc" => Some(0x1B),
        "tab" => Some(0x09),
        "enter" | "return" => Some(0x0D),
        _ => {
            let c = key.chars().next()?;
            if c.is_ascii_digit() {
                Some(c as u32)
            } else if c.is_ascii_alphabetic() {
                Some(c.to_ascii_uppercase() as u32)
            } else {
                None
            }
        }
    }
}

fn parse_action(spec: &str) -> Option<Action> {
    let mut parts = spec.split_whitespace();
    let verb = parts.next()?.to_ascii_lowercase();
    match verb.as_str() {
        "switch" => parts.next()?.parse::<i32>().ok().map(|n| Action::Switch(n - 1)),
        "move" => parts.next()?.parse::<i32>().ok().map(|n| Action::Move(n - 1)),
        "focus" => parts.next().and_then(parse_dir).map(Action::Focus),
        "reorder" => parts.next().and_then(parse_dir).map(Action::Reorder),
        "snap" => parts.next().and_then(parse_dir).map(Action::Snap),
        "float" => Some(Action::Float),
        "overlay" => Some(Action::Overlay),
        "close" => Some(Action::Close),
        "maximize" => Some(Action::Maximize),
        "quit" => Some(Action::Quit),
        "run" => {
            let rest = parts.collect::<Vec<_>>().join(" ");
            if rest.is_empty() { None } else { Some(Action::Run(rest)) }
        }
        _ => None,
    }
}

fn parse_dir(s: &str) -> Option<Direction> {
    match s.to_ascii_lowercase().as_str() {
        "left" => Some(Direction::Left),
        "right" => Some(Direction::Right),
        "up" => Some(Direction::Up),
        "down" => Some(Direction::Down),
        _ => None,
    }
}
