//! Windows backend (Win32 via `windows-sys`).
//!
//! - Discovery: `EnumWindows`, keeping visible, uncloaked, unowned, captioned top-level windows.
//! - Events: an out-of-context `SetWinEventHook`. The callback runs on this thread while it
//!   waits in `GetMessageW`; it queues the raw event and posts `WM_APP` so the loop wakes up.
//! - Hotkeys: `RegisterHotKey` on the thread queue; `WM_HOTKEY.wParam` is the binding index.
//! - Placement: `SetWindowPos`, corrected for the invisible resize borders Windows 10/11 add
//!   (`DWMWA_EXTENDED_FRAME_BOUNDS`), so gaps look even. The process is per-monitor DPI aware.
//! - Workspaces: `ShowWindow(SW_HIDE)`. Every window hidden this way is shown again when the
//!   backend is dropped, so quitting (or a panic) never strands windows.

use super::*;
use std::{cell::RefCell, collections::HashSet, ffi::c_void, mem, ptr};
use windows_sys::Win32::{
    Foundation::{BOOL, HWND, LPARAM, RECT, TRUE},
    Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS},
    System::Threading::{GetCurrentProcessId, GetCurrentThreadId},
    UI::{
        Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK},
        HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
        Input::KeyboardAndMouse::{
            keybd_event, RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, KEYEVENTF_KEYUP,
            MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, VK_MENU,
        },
        WindowsAndMessaging::*,
    },
};

const OBJID_WINDOW: i32 = 0;
const CHILDID_SELF: i32 = 0;

thread_local! {
    /// Raw (event, hwnd) pairs queued by the WinEvent callback, drained by the message loop.
    static QUEUE: RefCell<Vec<(u32, HWND)>> = const { RefCell::new(Vec::new()) };
}

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if hwnd.is_null() || id_object != OBJID_WINDOW || id_child != CHILDID_SELF {
        return;
    }
    QUEUE.with(|q| q.borrow_mut().push((event, hwnd)));
    PostThreadMessageW(GetCurrentThreadId(), WM_APP, 0, 0);
}

fn id(hwnd: HWND) -> WindowId {
    WindowId(hwnd as usize as u64)
}

fn hwnd(id: WindowId) -> HWND {
    id.0 as usize as HWND
}

fn wide_to_string(buf: &[u16], len: i32) -> String {
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

fn title(h: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(h, buf.as_mut_ptr(), buf.len() as i32) };
    wide_to_string(&buf, n)
}

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(h, buf.as_mut_ptr(), buf.len() as i32) };
    wide_to_string(&buf, n)
}

fn is_cloaked(h: HWND) -> bool {
    let mut cloaked: u32 = 0;
    let hr = unsafe {
        DwmGetWindowAttribute(
            h,
            DWMWA_CLOAKED as u32,
            &mut cloaked as *mut _ as *mut c_void,
            4,
        )
    };
    hr == 0 && cloaked != 0
}

/// Map a config key name (already lowercased) to a Win32 virtual-key code.
fn vk_code(key: &str) -> Option<u32> {
    let b = key.as_bytes();
    if b.len() == 1 && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) {
        return Some(b[0].to_ascii_uppercase() as u32);
    }
    if let Some(n) = key.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        return (1..=24).contains(&n).then_some(0x70 + n - 1);
    }
    Some(match key {
        "enter" | "return" => 0x0D,
        "space" => 0x20,
        "tab" => 0x09,
        "escape" | "esc" => 0x1B,
        "backspace" => 0x08,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        ";" | "semicolon" => 0xBA,
        "=" | "equal" => 0xBB,
        "," | "comma" => 0xBC,
        "-" | "minus" => 0xBD,
        "." | "period" => 0xBE,
        "/" | "slash" => 0xBF,
        "`" | "grave" => 0xC0,
        "[" => 0xDB,
        "\\" | "backslash" => 0xDC,
        "]" => 0xDD,
        "'" | "quote" => 0xDE,
        _ => return None,
    })
}

pub struct Win32Backend {
    float_rules: Vec<String>,
    /// Windows we hid for workspace switching. Their HIDE/SHOW events are ours, not the app's.
    hidden_by_us: HashSet<isize>,
    own_pid: u32,
}

impl Win32Backend {
    pub fn new() -> Self {
        unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        Self {
            float_rules: vec![],
            hidden_by_us: HashSet::new(),
            own_pid: unsafe { GetCurrentProcessId() },
        }
    }

    /// Should this window be tiled? Ordinary app windows only.
    fn is_manageable(&self, h: HWND) -> bool {
        unsafe {
            if IsWindowVisible(h) == 0 || IsIconic(h) != 0 || is_cloaked(h) {
                return false;
            }
            if !GetWindow(h, GW_OWNER).is_null() {
                return false;
            }
            let style = GetWindowLongW(h, GWL_STYLE) as u32;
            let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
            if style & WS_CHILD != 0 || style & WS_CAPTION != WS_CAPTION {
                return false;
            }
            if ex & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) != 0 {
                return false;
            }
            let mut pid = 0;
            GetWindowThreadProcessId(h, &mut pid);
            if pid == self.own_pid {
                return false;
            }
        }
        let t = title(h);
        if t.is_empty() {
            return false;
        }
        let class = class_name(h);
        !self
            .float_rules
            .iter()
            .any(|r| class.eq_ignore_ascii_case(r) || t.to_lowercase().contains(&r.to_lowercase()))
    }

    fn place(&self, h: HWND, r: Rect) {
        unsafe {
            if IsZoomed(h) != 0 {
                ShowWindow(h, SW_RESTORE);
            }
            // Visible frame vs. full window rect: the difference is the invisible border.
            let mut win: RECT = mem::zeroed();
            let mut vis: RECT = mem::zeroed();
            GetWindowRect(h, &mut win);
            let ok = DwmGetWindowAttribute(
                h,
                DWMWA_EXTENDED_FRAME_BOUNDS as u32,
                &mut vis as *mut _ as *mut c_void,
                mem::size_of::<RECT>() as u32,
            ) == 0;
            let (l, t, rr, b) = if ok {
                (
                    vis.left - win.left,
                    vis.top - win.top,
                    win.right - vis.right,
                    win.bottom - vis.bottom,
                )
            } else {
                (0, 0, 0, 0)
            };
            SetWindowPos(
                h,
                ptr::null_mut(),
                r.x - l,
                r.y - t,
                r.w + l + rr,
                r.h + t + b,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
    }

    fn focus(&self, h: HWND) {
        unsafe {
            if SetForegroundWindow(h) == 0 {
                // Windows only lets the process that got the last input steal focus.
                // A synthetic Alt tap satisfies that rule.
                keybd_event(VK_MENU as u8, 0, 0, 0);
                keybd_event(VK_MENU as u8, 0, KEYEVENTF_KEYUP, 0);
                SetForegroundWindow(h);
            }
        }
    }

    /// Turn a raw WinEvent into a core event, filtering out noise and our own hide/show.
    fn translate(&mut self, event: u32, h: HWND) -> Option<Event> {
        let key = h as isize;
        match event {
            EVENT_OBJECT_SHOW | EVENT_OBJECT_UNCLOAKED | EVENT_SYSTEM_MINIMIZEEND => {
                if self.hidden_by_us.contains(&key) {
                    return None;
                }
                self.is_manageable(h).then(|| Event::WindowCreated(id(h)))
            }
            EVENT_OBJECT_HIDE | EVENT_OBJECT_CLOAKED | EVENT_SYSTEM_MINIMIZESTART => {
                (!self.hidden_by_us.contains(&key)).then(|| Event::WindowDestroyed(id(h)))
            }
            EVENT_OBJECT_DESTROY => {
                self.hidden_by_us.remove(&key);
                Some(Event::WindowDestroyed(id(h)))
            }
            EVENT_SYSTEM_FOREGROUND => Some(Event::WindowFocused(id(h))),
            _ => None,
        }
    }

    fn register_hotkeys(&self, bindings: &[(Hotkey, Command)]) -> Vec<i32> {
        let mut ids = vec![];
        for (i, (hk, _)) in bindings.iter().enumerate() {
            let Some(vk) = vk_code(&hk.key) else {
                eprintln!("warning: unsupported key `{}`, binding skipped", hk.key);
                continue;
            };
            let mut mods: HOT_KEY_MODIFIERS = MOD_NOREPEAT;
            if hk.mods.alt {
                mods |= MOD_ALT;
            }
            if hk.mods.ctrl {
                mods |= MOD_CONTROL;
            }
            if hk.mods.shift {
                mods |= MOD_SHIFT;
            }
            if hk.mods.super_ {
                mods |= MOD_WIN;
            }
            if unsafe { RegisterHotKey(ptr::null_mut(), i as i32, mods, vk) } == 0 {
                eprintln!(
                    "warning: {:?}+{} is already taken by another program",
                    hk.mods, hk.key
                );
            } else {
                ids.push(i as i32);
            }
        }
        ids
    }
}

impl Drop for Win32Backend {
    fn drop(&mut self) {
        for h in self.hidden_by_us.drain() {
            unsafe { ShowWindow(h as HWND, SW_SHOWNA) };
        }
    }
}

impl Backend for Win32Backend {
    fn work_area(&self) -> Rect {
        let mut r: RECT = unsafe { mem::zeroed() };
        unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut r as *mut _ as *mut c_void, 0) };
        Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
    }

    fn existing_windows(&self) -> Vec<WindowId> {
        unsafe extern "system" fn collect(h: HWND, lp: LPARAM) -> BOOL {
            (*(lp as *mut Vec<HWND>)).push(h);
            TRUE
        }
        let mut all: Vec<HWND> = vec![];
        unsafe { EnumWindows(Some(collect), &mut all as *mut _ as LPARAM) };
        // EnumWindows is top-to-bottom z-order; reverse so the frontmost window ends up focused.
        all.into_iter()
            .rev()
            .filter(|&h| self.is_manageable(h))
            .map(id)
            .collect()
    }

    fn set_float_rules(&mut self, rules: &[String]) {
        self.float_rules = rules.to_vec();
    }

    fn apply(&mut self, action: &Action) {
        match *action {
            Action::Place(w, r) => self.place(hwnd(w), r),
            Action::Hide(w) => {
                self.hidden_by_us.insert(hwnd(w) as isize);
                unsafe { ShowWindow(hwnd(w), SW_HIDE) };
            }
            Action::Show(w) => {
                unsafe { ShowWindow(hwnd(w), SW_SHOWNA) };
                // The SHOW event this causes arrives later and becomes a WindowCreated, which the
                // core ignores because the window already belongs to a workspace.
                self.hidden_by_us.remove(&(hwnd(w) as isize));
            }
            Action::Focus(w) => self.focus(hwnd(w)),
            Action::Quit => unsafe { PostQuitMessage(0) },
        }
    }

    fn run(
        &mut self,
        bindings: &[(Hotkey, Command)],
        handler: &mut Handler<'_>,
    ) -> Result<(), String> {
        let hotkeys = self.register_hotkeys(bindings);
        if hotkeys.is_empty() && !bindings.is_empty() {
            return Err("no hotkeys could be registered".into());
        }
        let hooks: Vec<HWINEVENTHOOK> = [
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
            (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
            (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE),
            (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
        ]
        .iter()
        .map(|&(lo, hi)| unsafe {
            SetWinEventHook(
                lo,
                hi,
                ptr::null_mut(),
                Some(win_event_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        })
        .collect();

        let mut msg: MSG = unsafe { mem::zeroed() };
        'outer: while unsafe { GetMessageW(&mut msg, ptr::null_mut(), 0, 0) } > 0 {
            let mut events = vec![];
            if msg.message == WM_HOTKEY {
                if let Some((_, cmd)) = bindings.get(msg.wParam) {
                    events.push(Event::Command(*cmd));
                }
            }
            for (ev, h) in QUEUE.with(|q| mem::take(&mut *q.borrow_mut())) {
                events.extend(self.translate(ev, h));
            }
            for ev in events {
                for action in handler(ev) {
                    self.apply(&action);
                    if action == Action::Quit {
                        break 'outer;
                    }
                }
            }
        }

        for h in hooks {
            unsafe { UnhookWinEvent(h) };
        }
        for i in hotkeys {
            unsafe { UnregisterHotKey(ptr::null_mut(), i) };
        }
        Ok(())
    }
}
