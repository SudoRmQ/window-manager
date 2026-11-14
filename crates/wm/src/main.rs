mod ipc_server;

use std::{env, fs, path::PathBuf, sync::mpsc};
use wm_core::{
    config::Config,
    manager::{Action, Event, Manager},
};

const USAGE: &str = "usage:
  wm [config.toml]                 run the window manager
  wm msg [-t type] [command...]    send an i3 command or query to a running wm
                                   (types: command, get_tree, get_workspaces, get_outputs, get_version)";

fn config_path(arg: Option<&String>) -> Option<PathBuf> {
    if let Some(p) = arg {
        return Some(p.into());
    }
    let base = if cfg!(windows) {
        env::var_os("APPDATA").map(PathBuf::from)
    } else {
        env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
    };
    base.map(|b| b.join("tiler").join("config.toml"))
}

fn load_config(arg: Option<&String>) -> Result<Config, String> {
    match config_path(arg) {
        Some(p) if p.exists() => {
            let src = fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            Config::from_toml(&src).map_err(|e| format!("{}: {e}", p.display()))
        }
        Some(p) if arg.is_some() => Err(format!("{}: file not found", p.display())),
        _ => Ok(Config::default()),
    }
}

fn run(config_arg: Option<&String>) -> Result<(), String> {
    let config = load_config(config_arg).map_err(|e| format!("config error: {e}"))?;

    let mut backend = wm_platform::native();
    backend.set_float_rules(&config.float_rules);
    let mut manager = Manager::new(&config, backend.work_area());

    let (tx, rx) = mpsc::channel::<ipc_server::Request>();
    let _ipc = ipc_server::start(tx, backend.waker())
        .map_err(|e| format!("IPC socket {}: {e}", ipc_server::socket_path().display()))?;

    for w in backend.existing_windows() {
        for a in manager.handle(Event::WindowCreated(w)) {
            backend.apply(&a);
        }
    }

    let mut handler = |ev: Event| -> Vec<Action> {
        match ev {
            Event::Wake => {
                let mut actions = vec![];
                while let Ok(req) = rx.try_recv() {
                    let (reply, a) = manager.ipc(req.ty, &req.payload);
                    actions.extend(a);
                    let _ = req.reply.send(reply);
                }
                actions
            }
            ev => manager.handle(ev),
        }
    };
    backend.run(&config.bindings, &mut handler)
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("msg") => ipc_server::client(&args[1..]).map(|r| println!("{r}")),
        Some("-h" | "--help") => {
            println!("{USAGE}");
            Ok(())
        }
        _ => run(args.first()),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
