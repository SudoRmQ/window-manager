//! IPC transport: a Unix socket (macOS, Linux) or a named pipe (Windows), speaking i3's
//! protocol (see `wm_core::ipc`). Each client gets its own thread; requests are forwarded to
//! the main thread, which owns the window manager, and answered from there.

use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::mpsc,
    thread,
};
use wm_core::ipc;
use wm_platform::Waker;

pub struct Request {
    pub ty: u32,
    pub payload: Vec<u8>,
    pub reply: mpsc::Sender<String>,
}

/// Where the server listens. `TILER_SOCKET` overrides the default.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("TILER_SOCKET") {
        return p.into();
    }
    #[cfg(windows)]
    {
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
        PathBuf::from(format!(r"\\.\pipe\tiler-{user}"))
    }
    #[cfg(not(windows))]
    {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .or_else(|| std::env::var_os("TMPDIR"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
        dir.join(format!("tiler-{user}.sock"))
    }
}

fn serve_client(mut conn: impl Read + Write, tx: mpsc::Sender<Request>, wake: Waker) {
    while let Ok((ty, payload)) = ipc::read_message(&mut conn) {
        let (reply_tx, reply_rx) = mpsc::channel();
        if tx
            .send(Request {
                ty,
                payload,
                reply: reply_tx,
            })
            .is_err()
        {
            return;
        }
        wake();
        let Ok(reply) = reply_rx.recv() else { return };
        if ipc::write_message(&mut conn, ty, reply.as_bytes()).is_err() {
            return;
        }
    }
}

/// Start listening in the background. Returns a guard that removes the socket file on drop.
#[cfg(not(windows))]
pub fn start(tx: mpsc::Sender<Request>, wake: Waker) -> std::io::Result<impl Drop> {
    use std::os::unix::{fs::PermissionsExt, net::UnixListener};
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let path = socket_path();
    // A stale socket from a crashed run would make bind fail.
    if std::os::unix::net::UnixStream::connect(&path).is_err() {
        let _ = std::fs::remove_file(&path);
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let (tx, wake) = (tx.clone(), wake.clone());
            thread::spawn(move || serve_client(conn, tx, wake));
        }
    });
    Ok(Cleanup(path))
}

#[cfg(windows)]
pub fn start(tx: mpsc::Sender<Request>, wake: Waker) -> std::io::Result<impl Drop> {
    use std::{fs::File, os::windows::io::FromRawHandle, ptr};
    use windows_sys::Win32::{
        Foundation::{GetLastError, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE},
        Storage::FileSystem::PIPE_ACCESS_DUPLEX,
        System::Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        },
    };
    struct Nothing;
    impl Drop for Nothing {
        fn drop(&mut self) {}
    }
    let name: Vec<u16> = socket_path()
        .to_string_lossy()
        .encode_utf16()
        .chain([0])
        .collect();
    let create = move || unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            64 * 1024,
            64 * 1024,
            0,
            ptr::null(),
        )
    };
    // Create the first instance now so errors (e.g. already running) surface immediately.
    let first = create();
    if first == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    let first = first as usize;
    thread::spawn(move || {
        let mut next = Some(first);
        loop {
            let h = match next.take() {
                Some(h) => h as _,
                None => create(),
            };
            if h == INVALID_HANDLE_VALUE {
                return;
            }
            let ok = unsafe { ConnectNamedPipe(h, ptr::null_mut()) } != 0
                || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
            if !ok {
                continue;
            }
            // File takes ownership of the handle and closes it when the client is done.
            let file = unsafe { File::from_raw_handle(h as _) };
            let (tx, wake) = (tx.clone(), wake.clone());
            thread::spawn(move || serve_client(file, tx, wake));
        }
    });
    Ok(Nothing)
}

/// `wm msg [-t type] [payload...]`, like `i3-msg`.
pub fn client(args: &[String]) -> Result<String, String> {
    let mut ty = ipc::RUN_COMMAND;
    let mut rest = args;
    if let [flag, name, tail @ ..] = args {
        if flag == "-t" {
            ty = ipc::type_from_name(name).ok_or(format!("unknown message type `{name}`"))?;
            rest = tail;
        }
    }
    let payload = rest.join(" ");
    let path = socket_path();
    let fail = |e: std::io::Error| format!("{}: {e} (is wm running?)", path.display());
    #[cfg(windows)]
    let mut conn = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(fail)?;
    #[cfg(not(windows))]
    let mut conn = std::os::unix::net::UnixStream::connect(&path).map_err(fail)?;
    ipc::write_message(&mut conn, ty, payload.as_bytes()).map_err(|e| e.to_string())?;
    let (_, reply) = ipc::read_message(&mut conn).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&reply).into_owned())
}
