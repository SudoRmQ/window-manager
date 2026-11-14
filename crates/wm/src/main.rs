use std::{env, fs, path::PathBuf};
use wm_core::{config::Config, manager::{Action, Event, Manager}};

fn config_path() -> Option<PathBuf> {
    if let Some(p) = env::args().nth(1) {
        return Some(p.into());
    }
    let base = if cfg!(windows) {
        env::var_os("APPDATA").map(PathBuf::from)
    } else {
        env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
    };
    base.map(|b| b.join("tiler").join("config.toml"))
}

fn load_config() -> Result<Config, String> {
    match config_path() {
        Some(p) if p.exists() => {
            let src = fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            Config::from_toml(&src).map_err(|e| format!("{}: {e}", p.display()))
        }
        _ => Ok(Config::default()),
    }
}

fn main() {
    let config = match load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("config error: {e}");
            std::process::exit(1);
        }
    };

    let mut backend = wm_platform::native();
    let mut manager = Manager::new(&config, backend.work_area());

    for w in backend.existing_windows() {
        if backend.matches_rule(w, &config.float_rules) {
            continue;
        }
        for a in manager.handle(Event::WindowCreated(w)) {
            backend.apply(&a);
        }
    }

    let mut handler = |ev: Event| -> Vec<Action> { manager.handle(ev) };
    if let Err(e) = backend.run(&config.bindings, &mut handler) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
