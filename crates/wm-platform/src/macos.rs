//! macOS backend (Accessibility API, CoreGraphics event tap, CFRunLoop).
//!
//! - Permission: `AXIsProcessTrustedWithOptions` prompts once; the user must allow the binary
//!   (or the terminal running it) under System Settings > Privacy & Security > Accessibility.
//! - Discovery: `CGWindowListCopyWindowInfo` finds pids that own normal on-screen windows; each
//!   app's `AXWindows` are kept when their subrole is `AXStandardWindow`. WindowId is the
//!   CGWindowID from the private but long-stable `_AXUIElementGetWindow`.
//! - Events: one `AXObserver` per app (window created, focus changed) plus per-window
//!   notifications (destroyed, minimised, restored). A 1 s run-loop timer picks up newly
//!   launched and quit apps, and any window whose notification was missed.
//! - Hotkeys: an active `CGEventTap` on key-down that swallows matching chords.
//! - Placement: `AXPosition` then `AXSize` (top-left origin on the primary display).
//! - Workspaces: macOS has no public API for Spaces or hiding single windows, so hidden
//!   windows are parked in the bottom-right corner of the screen (as AeroSpace does), and are
//!   moved back on screen when the backend is dropped.
//!
//! Everything runs on the main thread inside `CFRunLoopRun`.

#![allow(non_upper_case_globals, non_snake_case, clippy::upper_case_acronyms)]

use super::*;
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    ffi::{c_char, c_void, CString},
    ptr,
};

// ---------------------------------------------------------------------------------------------
// FFI
// ---------------------------------------------------------------------------------------------

type CFTypeRef = *const c_void;
type CFStringRef = CFTypeRef;
type CFArrayRef = CFTypeRef;
type CFDictionaryRef = CFTypeRef;
type CFRunLoopRef = CFTypeRef;
type CFRunLoopSourceRef = CFTypeRef;
type CFRunLoopTimerRef = CFTypeRef;
type CFMachPortRef = CFTypeRef;
type AXUIElementRef = CFTypeRef;
type AXObserverRef = CFTypeRef;
type AXValueRef = CFTypeRef;
type CGEventRef = *mut c_void;
type AXError = i32;
type Boolean = u8;

const kCFStringEncodingUTF8: u32 = 0x0800_0100;
const kCFNumberSInt64Type: i32 = 4;
const kAXValueCGPointType: u32 = 1;
const kAXValueCGSizeType: u32 = 2;
const kAXErrorSuccess: AXError = 0;
const kCGWindowListOptionOnScreenOnly: u32 = 1;
const kCGWindowListExcludeDesktopElements: u32 = 16;
const kCGSessionEventTap: u32 = 1;
const kCGHeadInsertEventTap: u32 = 0;
const kCGEventTapOptionDefault: u32 = 0;
const kCGEventKeyDown: u32 = 10;
const kCGEventTapDisabledByTimeout: u32 = 0xFFFF_FFFE;
const kCGEventTapDisabledByUserInput: u32 = 0xFFFF_FFFF;
const kCGKeyboardEventKeycode: u32 = 9;
const FLAG_SHIFT: u64 = 0x0002_0000;
const FLAG_CTRL: u64 = 0x0004_0000;
const FLAG_ALT: u64 = 0x0008_0000;
const FLAG_CMD: u64 = 0x0010_0000;
const FLAG_MASK: u64 = FLAG_SHIFT | FLAG_CTRL | FLAG_ALT | FLAG_CMD;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

unsafe impl objc2::Encode for CGPoint {
    const ENCODING: objc2::Encoding =
        objc2::Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl objc2::Encode for CGSize {
    const ENCODING: objc2::Encoding =
        objc2::Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl objc2::Encode for CGRect {
    const ENCODING: objc2::Encoding =
        objc2::Encoding::Struct("CGRect", &[CGPoint::ENCODING, CGSize::ENCODING]);
}

#[repr(C)]
struct CFRunLoopTimerContext {
    version: isize,
    info: *mut c_void,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
}

type AXObserverCallback =
    unsafe extern "C" fn(AXObserverRef, AXUIElementRef, CFStringRef, *mut c_void);
type CGEventTapCallBack =
    unsafe extern "C" fn(*mut c_void, u32, CGEventRef, *mut c_void) -> CGEventRef;
type CFRunLoopTimerCallBack = unsafe extern "C" fn(CFRunLoopTimerRef, *mut c_void);

#[repr(C)]
struct CFRunLoopSourceContext {
    version: isize,
    info: *mut c_void,
    retain: *const c_void,
    release: *const c_void,
    copy_description: *const c_void,
    equal: *const c_void,
    hash: *const c_void,
    schedule: *const c_void,
    cancel: *const c_void,
    perform: unsafe extern "C" fn(*mut c_void),
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopDefaultMode: CFStringRef;
    static kCFBooleanTrue: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    fn CFRelease(cf: CFTypeRef);
    fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    fn CFEqual(a: CFTypeRef, b: CFTypeRef) -> Boolean;
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFStringCreateWithCString(alloc: CFTypeRef, s: *const c_char, enc: u32) -> CFStringRef;
    fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, size: isize, enc: u32) -> Boolean;
    fn CFArrayGetCount(a: CFArrayRef) -> isize;
    fn CFArrayGetValueAtIndex(a: CFArrayRef, i: isize) -> CFTypeRef;
    fn CFDictionaryGetValue(d: CFDictionaryRef, key: CFTypeRef) -> CFTypeRef;
    fn CFDictionaryCreate(
        alloc: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        n: isize,
        key_cb: *const c_void,
        value_cb: *const c_void,
    ) -> CFDictionaryRef;
    fn CFNumberGetValue(n: CFTypeRef, ty: i32, out: *mut c_void) -> Boolean;
    fn CFRunLoopGetMain() -> CFRunLoopRef;
    fn CFRunLoopRun();
    fn CFRunLoopStop(rl: CFRunLoopRef);
    fn CFRunLoopAddSource(rl: CFRunLoopRef, src: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRemoveSource(rl: CFRunLoopRef, src: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopAddTimer(rl: CFRunLoopRef, t: CFRunLoopTimerRef, mode: CFStringRef);
    fn CFRunLoopTimerInvalidate(t: CFRunLoopTimerRef);
    fn CFRunLoopTimerCreate(
        alloc: CFTypeRef,
        fire: f64,
        interval: f64,
        flags: usize,
        order: isize,
        cb: CFRunLoopTimerCallBack,
        ctx: *mut CFRunLoopTimerContext,
    ) -> CFRunLoopTimerRef;
    fn CFAbsoluteTimeGetCurrent() -> f64;
    fn CFRunLoopSourceCreate(
        alloc: CFTypeRef,
        order: isize,
        ctx: *mut CFRunLoopSourceContext,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopSourceSignal(src: CFRunLoopSourceRef);
    fn CFRunLoopWakeUp(rl: CFRunLoopRef);
    fn CFMachPortCreateRunLoopSource(
        alloc: CFTypeRef,
        port: CFMachPortRef,
        order: isize,
    ) -> CFRunLoopSourceRef;
    fn CFMachPortInvalidate(port: CFMachPortRef);
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> Boolean;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyAttributeValue(
        el: AXUIElementRef,
        attr: CFStringRef,
        out: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementSetAttributeValue(el: AXUIElementRef, attr: CFStringRef, v: CFTypeRef)
        -> AXError;
    fn AXUIElementPerformAction(el: AXUIElementRef, action: CFStringRef) -> AXError;
    fn AXUIElementSetMessagingTimeout(el: AXUIElementRef, secs: f32) -> AXError;
    fn AXValueCreate(ty: u32, v: *const c_void) -> AXValueRef;
    fn AXObserverCreate(pid: i32, cb: AXObserverCallback, out: *mut AXObserverRef) -> AXError;
    fn AXObserverAddNotification(
        o: AXObserverRef,
        el: AXUIElementRef,
        n: CFStringRef,
        refcon: *mut c_void,
    ) -> AXError;
    fn AXObserverGetRunLoopSource(o: AXObserverRef) -> CFRunLoopSourceRef;
    fn _AXUIElementGetWindow(el: AXUIElementRef, out: *mut u32) -> AXError;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFArrayRef;
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        mask: u64,
        cb: CGEventTapCallBack,
        info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(e: CGEventRef, field: u32) -> i64;
    fn CGEventGetFlags(e: CGEventRef) -> u64;
}

#[link(name = "AppKit", kind = "framework")]
extern "C" {}

// ---------------------------------------------------------------------------------------------
// Small CF helpers
// ---------------------------------------------------------------------------------------------

/// Owned CFString, released on drop.
struct CfStr(CFStringRef);

impl CfStr {
    fn new(s: &str) -> Self {
        let c = CString::new(s).expect("no interior NUL");
        Self(unsafe { CFStringCreateWithCString(ptr::null(), c.as_ptr(), kCFStringEncodingUTF8) })
    }
}

impl Drop for CfStr {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

fn cf_to_string(s: CFTypeRef) -> Option<String> {
    if s.is_null() || unsafe { CFGetTypeID(s) != CFStringGetTypeID() } {
        return None;
    }
    let mut buf = vec![0 as c_char; 1024];
    let ok = unsafe {
        CFStringGetCString(
            s,
            buf.as_mut_ptr(),
            buf.len() as isize,
            kCFStringEncodingUTF8,
        )
    };
    (ok != 0).then(|| {
        unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    })
}

/// Copy an attribute; the caller owns the result (+1) and must release it.
fn ax_copy(el: AXUIElementRef, attr: &str) -> Option<CFTypeRef> {
    let mut out: CFTypeRef = ptr::null();
    let a = CfStr::new(attr);
    let err = unsafe { AXUIElementCopyAttributeValue(el, a.0, &mut out) };
    (err == kAXErrorSuccess && !out.is_null()).then_some(out)
}

fn ax_string(el: AXUIElementRef, attr: &str) -> Option<String> {
    let v = ax_copy(el, attr)?;
    let s = cf_to_string(v);
    unsafe { CFRelease(v) };
    s
}

fn ax_is_true(el: AXUIElementRef, attr: &str) -> bool {
    ax_copy(el, attr).is_some_and(|v| {
        let t = unsafe { CFEqual(v, kCFBooleanTrue) } != 0;
        unsafe { CFRelease(v) };
        t
    })
}

fn ax_set(el: AXUIElementRef, attr: &str, v: CFTypeRef) {
    let a = CfStr::new(attr);
    unsafe { AXUIElementSetAttributeValue(el, a.0, v) };
}

fn window_number(el: AXUIElementRef) -> Option<u64> {
    let mut id = 0u32;
    (unsafe { _AXUIElementGetWindow(el, &mut id) } == kAXErrorSuccess && id != 0)
        .then_some(id as u64)
}

/// Pids owning normal-layer on-screen windows, i.e. regular apps the user can see.
fn app_pids() -> HashSet<i32> {
    let mut pids = HashSet::new();
    unsafe {
        let list = CGWindowListCopyWindowInfo(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
            0,
        );
        if list.is_null() {
            return pids;
        }
        let k_pid = CfStr::new("kCGWindowOwnerPID");
        let k_layer = CfStr::new("kCGWindowLayer");
        for i in 0..CFArrayGetCount(list) {
            let d = CFArrayGetValueAtIndex(list, i);
            let (mut pid, mut layer) = (0i64, -1i64);
            let p = CFDictionaryGetValue(d, k_pid.0);
            let l = CFDictionaryGetValue(d, k_layer.0);
            if !p.is_null() && !l.is_null() {
                CFNumberGetValue(p, kCFNumberSInt64Type, &mut pid as *mut _ as *mut c_void);
                CFNumberGetValue(l, kCFNumberSInt64Type, &mut layer as *mut _ as *mut c_void);
                if layer == 0 {
                    pids.insert(pid as i32);
                }
            }
        }
        CFRelease(list);
    }
    pids
}

/// Map a config key name (lowercased) to a macOS virtual key code (kVK_ANSI_*).
fn key_code(key: &str) -> Option<i64> {
    const LETTERS: &[(char, i64)] = &[
        ('a', 0),
        ('s', 1),
        ('d', 2),
        ('f', 3),
        ('h', 4),
        ('g', 5),
        ('z', 6),
        ('x', 7),
        ('c', 8),
        ('v', 9),
        ('b', 11),
        ('q', 12),
        ('w', 13),
        ('e', 14),
        ('r', 15),
        ('y', 16),
        ('t', 17),
        ('1', 18),
        ('2', 19),
        ('3', 20),
        ('4', 21),
        ('6', 22),
        ('5', 23),
        ('=', 24),
        ('9', 25),
        ('7', 26),
        ('-', 27),
        ('8', 28),
        ('0', 29),
        (']', 30),
        ('o', 31),
        ('u', 32),
        ('[', 33),
        ('i', 34),
        ('p', 35),
        ('l', 37),
        ('j', 38),
        ('\'', 39),
        ('k', 40),
        (';', 41),
        ('\\', 42),
        (',', 43),
        ('/', 44),
        ('n', 45),
        ('m', 46),
        ('.', 47),
        ('`', 50),
    ];
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return LETTERS.iter().find(|(k, _)| *k == c).map(|(_, v)| *v);
    }
    const FKEYS: [i64; 12] = [122, 120, 99, 118, 96, 97, 98, 100, 101, 109, 103, 111];
    if let Some(n) = key.strip_prefix('f').and_then(|n| n.parse::<usize>().ok()) {
        return FKEYS.get(n.wrapping_sub(1)).copied();
    }
    Some(match key {
        "enter" | "return" => 36,
        "tab" => 48,
        "space" => 49,
        "backspace" | "delete" => 51,
        "escape" | "esc" => 53,
        "left" => 123,
        "right" => 124,
        "down" => 125,
        "up" => 126,
        "equal" => 24,
        "minus" => 27,
        "comma" => 43,
        "period" => 47,
        "slash" => 44,
        "semicolon" => 41,
        "quote" => 39,
        "backslash" => 42,
        "grave" => 50,
        _ => return None,
    })
}

fn chord_flags(h: &Hotkey) -> u64 {
    let m = h.mods;
    (if m.shift { FLAG_SHIFT } else { 0 })
        | (if m.ctrl { FLAG_CTRL } else { 0 })
        | (if m.alt { FLAG_ALT } else { 0 })
        | (if m.super_ { FLAG_CMD } else { 0 })
}

// ---------------------------------------------------------------------------------------------
// Backend
// ---------------------------------------------------------------------------------------------

struct App {
    element: AXUIElementRef,
    observer: AXObserverRef,
    name: String,
}

struct Win {
    pid: i32,
    element: AXUIElementRef,
}

pub struct AxBackend {
    trusted: bool,
    float_rules: Vec<String>,
    apps: HashMap<i32, App>,
    windows: HashMap<u64, Win>,
    /// Windows we parked off-screen for a hidden workspace.
    parked: HashSet<u64>,
    chords: Vec<(i64, u64, Vec<Command>)>,
    tap: CFMachPortRef,
    /// Signalled by `waker()` from other threads; its perform callback runs on the main loop.
    wake_source: CFRunLoopSourceRef,
}

/// A CF object pointer that may cross threads (CFRunLoopSourceSignal/WakeUp are thread-safe).
struct SendPtr(CFTypeRef);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

impl SendPtr {
    // A method (not field access) so closures capture the whole Send wrapper.
    fn get(&self) -> CFTypeRef {
        self.0
    }
}

/// Raw notifications collected by C callbacks, translated by the backend.
enum Raw {
    Created(i32, AXUIElementRef),
    Destroyed(AXUIElementRef),
    Minimized(AXUIElementRef),
    Restored(AXUIElementRef),
    Focused(i32),
    Hotkey(Vec<Command>),
    Rescan,
    Wake,
}

/// The running context, reachable from C callbacks during `CFRunLoopRun`.
struct Ctx<'a, 'h> {
    backend: &'a mut AxBackend,
    handler: &'a mut Handler<'h>,
}

thread_local! {
    static CTX: Cell<*mut c_void> = const { Cell::new(ptr::null_mut()) };
}

fn dispatch(raw: Raw) {
    let p = CTX.with(Cell::get) as *mut Ctx;
    if p.is_null() {
        return;
    }
    // SAFETY: CTX is set only while `run` is blocked in CFRunLoopRun on this thread, and the
    // pointee lives on `run`'s stack for that whole time. Callbacks never nest.
    let ctx = unsafe { &mut *p };
    for ev in ctx.backend.translate(raw) {
        for action in (ctx.handler)(ev) {
            ctx.backend.apply(&action);
        }
    }
}

unsafe extern "C" fn ax_callback(
    _o: AXObserverRef,
    el: AXUIElementRef,
    n: CFStringRef,
    refcon: *mut c_void,
) {
    let pid = refcon as usize as i32;
    let raw = match cf_to_string(n).as_deref() {
        Some("AXWindowCreated") => Raw::Created(pid, el),
        Some("AXUIElementDestroyed") => Raw::Destroyed(el),
        Some("AXWindowMiniaturized") => Raw::Minimized(el),
        Some("AXWindowDeminiaturized") => Raw::Restored(el),
        Some("AXFocusedWindowChanged") | Some("AXApplicationActivated") => Raw::Focused(pid),
        _ => return,
    };
    dispatch(raw);
}

unsafe extern "C" fn tap_callback(
    _p: *mut c_void,
    ty: u32,
    e: CGEventRef,
    _info: *mut c_void,
) -> CGEventRef {
    let p = CTX.with(Cell::get) as *mut Ctx;
    if p.is_null() {
        return e;
    }
    if ty == kCGEventTapDisabledByTimeout || ty == kCGEventTapDisabledByUserInput {
        CGEventTapEnable((*p).backend.tap, true);
        return e;
    }
    if ty != kCGEventKeyDown {
        return e;
    }
    let code = CGEventGetIntegerValueField(e, kCGKeyboardEventKeycode);
    let flags = CGEventGetFlags(e) & FLAG_MASK;
    let hit = (*p)
        .backend
        .chords
        .iter()
        .find(|(k, f, _)| *k == code && *f == flags)
        .map(|c| c.2.clone());
    match hit {
        Some(cmd) => {
            dispatch(Raw::Hotkey(cmd));
            ptr::null_mut() // swallow the key so the focused app never sees it
        }
        None => e,
    }
}

unsafe extern "C" fn timer_callback(_t: CFRunLoopTimerRef, _info: *mut c_void) {
    dispatch(Raw::Rescan);
}

unsafe extern "C" fn wake_callback(_info: *mut c_void) {
    dispatch(Raw::Wake);
}

impl AxBackend {
    pub fn new() -> Self {
        let trusted = unsafe {
            let key = CfStr::new("AXTrustedCheckOptionPrompt");
            let opts = CFDictionaryCreate(
                ptr::null(),
                &key.0,
                &kCFBooleanTrue,
                1,
                &kCFTypeDictionaryKeyCallBacks as *const _,
                &kCFTypeDictionaryValueCallBacks as *const _,
            );
            let t = AXIsProcessTrustedWithOptions(opts) != 0;
            CFRelease(opts);
            t
        };
        let mut me = Self {
            trusted,
            float_rules: vec![],
            apps: HashMap::new(),
            windows: HashMap::new(),
            parked: HashSet::new(),
            chords: vec![],
            tap: ptr::null(),
            wake_source: unsafe {
                let mut ctx = CFRunLoopSourceContext {
                    version: 0,
                    info: ptr::null_mut(),
                    retain: ptr::null(),
                    release: ptr::null(),
                    copy_description: ptr::null(),
                    equal: ptr::null(),
                    hash: ptr::null(),
                    schedule: ptr::null(),
                    cancel: ptr::null(),
                    perform: wake_callback,
                };
                CFRunLoopSourceCreate(ptr::null(), 0, &mut ctx)
            },
        };
        if trusted {
            for pid in app_pids() {
                me.track_app(pid);
            }
        }
        me
    }

    /// Start observing an app. Returns false if it can't be observed (e.g. it is exiting).
    fn track_app(&mut self, pid: i32) -> bool {
        if self.apps.contains_key(&pid) || pid == std::process::id() as i32 {
            return self.apps.contains_key(&pid);
        }
        unsafe {
            let element = AXUIElementCreateApplication(pid);
            AXUIElementSetMessagingTimeout(element, 1.0);
            let mut observer: AXObserverRef = ptr::null();
            if AXObserverCreate(pid, ax_callback, &mut observer) != kAXErrorSuccess {
                CFRelease(element);
                return false;
            }
            let refcon = pid as usize as *mut c_void;
            for n in [
                "AXWindowCreated",
                "AXFocusedWindowChanged",
                "AXApplicationActivated",
            ] {
                let s = CfStr::new(n);
                AXObserverAddNotification(observer, element, s.0, refcon);
            }
            CFRunLoopAddSource(
                CFRunLoopGetMain(),
                AXObserverGetRunLoopSource(observer),
                kCFRunLoopDefaultMode,
            );
            let name = ax_string(element, "AXTitle").unwrap_or_default();
            self.apps.insert(
                pid,
                App {
                    element,
                    observer,
                    name,
                },
            );
        }
        true
    }

    fn untrack_app(&mut self, pid: i32) -> Vec<u64> {
        let gone: Vec<u64> = self
            .windows
            .iter()
            .filter(|(_, w)| w.pid == pid)
            .map(|(k, _)| *k)
            .collect();
        for id in &gone {
            self.forget_window(*id);
        }
        if let Some(app) = self.apps.remove(&pid) {
            unsafe {
                CFRunLoopRemoveSource(
                    CFRunLoopGetMain(),
                    AXObserverGetRunLoopSource(app.observer),
                    kCFRunLoopDefaultMode,
                );
                CFRelease(app.observer);
                CFRelease(app.element);
            }
        }
        gone
    }

    fn forget_window(&mut self, id: u64) {
        self.parked.remove(&id);
        if let Some(w) = self.windows.remove(&id) {
            unsafe { CFRelease(w.element) };
        }
    }

    fn is_manageable(&self, pid: i32, el: AXUIElementRef) -> bool {
        if ax_string(el, "AXSubrole").as_deref() != Some("AXStandardWindow")
            || ax_is_true(el, "AXMinimized")
        {
            return false;
        }
        let title = ax_string(el, "AXTitle").unwrap_or_default().to_lowercase();
        let app = self.apps.get(&pid).map(|a| a.name.as_str()).unwrap_or("");
        !self
            .float_rules
            .iter()
            .any(|r| app.eq_ignore_ascii_case(r) || title.contains(&r.to_lowercase()))
    }

    /// Start tracking a window element (borrowed). Returns its id if it should be tiled.
    fn track_window(&mut self, pid: i32, el: AXUIElementRef) -> Option<u64> {
        let id = window_number(el)?;
        if self.windows.contains_key(&id) {
            return None;
        }
        if !self.is_manageable(pid, el) {
            return None;
        }
        unsafe {
            CFRetain(el);
            if let Some(app) = self.apps.get(&pid) {
                let refcon = pid as usize as *mut c_void;
                for n in [
                    "AXUIElementDestroyed",
                    "AXWindowMiniaturized",
                    "AXWindowDeminiaturized",
                ] {
                    let s = CfStr::new(n);
                    AXObserverAddNotification(app.observer, el, s.0, refcon);
                }
            }
        }
        self.windows.insert(id, Win { pid, element: el });
        Some(id)
    }

    fn app_windows(&self, pid: i32) -> Vec<AXUIElementRef> {
        let Some(app) = self.apps.get(&pid) else {
            return vec![];
        };
        let Some(arr) = ax_copy(app.element, "AXWindows") else {
            return vec![];
        };
        let v = unsafe {
            (0..CFArrayGetCount(arr))
                .map(|i| CFArrayGetValueAtIndex(arr, i))
                .collect::<Vec<_>>()
        };
        // Keep the elements alive past the array.
        for e in &v {
            unsafe { CFRetain(*e) };
        }
        unsafe { CFRelease(arr) };
        v
    }

    /// Track every manageable window of `pid` not yet known; returns the new ids.
    fn scan_app(&mut self, pid: i32) -> Vec<u64> {
        let mut new = vec![];
        for el in self.app_windows(pid) {
            new.extend(self.track_window(pid, el));
            unsafe { CFRelease(el) };
        }
        new
    }

    fn id_of(&self, el: AXUIElementRef) -> Option<u64> {
        // Destroyed elements may no longer answer _AXUIElementGetWindow; compare elements instead.
        self.windows
            .iter()
            .find(|(_, w)| unsafe { CFEqual(w.element, el) } != 0)
            .map(|(k, _)| *k)
    }

    fn translate(&mut self, raw: Raw) -> Vec<Event> {
        let created = |ids: Vec<u64>| {
            ids.into_iter()
                .map(|i| Event::WindowCreated(WindowId(i)))
                .collect::<Vec<_>>()
        };
        match raw {
            Raw::Hotkey(c) => vec![Event::Commands(c)],
            Raw::Wake => vec![Event::Wake],
            Raw::Created(pid, el) => self
                .track_window(pid, el)
                .map(|i| Event::WindowCreated(WindowId(i)))
                .into_iter()
                .collect(),
            // A minimised window stays tracked but leaves the layout until it is restored.
            Raw::Restored(el) => self
                .id_of(el)
                .map(|i| Event::WindowCreated(WindowId(i)))
                .into_iter()
                .collect(),
            Raw::Minimized(el) => self
                .id_of(el)
                .map(|i| Event::WindowDestroyed(WindowId(i)))
                .into_iter()
                .collect(),
            Raw::Destroyed(el) => match self.id_of(el) {
                Some(id) => {
                    self.forget_window(id);
                    vec![Event::WindowDestroyed(WindowId(id))]
                }
                None => vec![],
            },
            Raw::Focused(pid) => {
                let Some(app) = self.apps.get(&pid) else {
                    return vec![];
                };
                let Some(w) = ax_copy(app.element, "AXFocusedWindow") else {
                    return vec![];
                };
                let id = window_number(w);
                unsafe { CFRelease(w) };
                id.filter(|i| self.windows.contains_key(i))
                    .map(|i| Event::WindowFocused(WindowId(i)))
                    .into_iter()
                    .collect()
            }
            Raw::Rescan => {
                let live = app_pids();
                let mut evs: Vec<Event> = vec![];
                let dead: Vec<i32> = self
                    .apps
                    .keys()
                    .copied()
                    .filter(|p| !live.contains(p))
                    .collect();
                for pid in dead {
                    // An app with no on-screen windows may still be running (all windows closed or
                    // hidden); only drop it when it has no AX windows left either.
                    if self.app_windows(pid).is_empty() {
                        evs.extend(
                            self.untrack_app(pid)
                                .into_iter()
                                .map(|i| Event::WindowDestroyed(WindowId(i))),
                        );
                    }
                }
                for pid in live {
                    if self.track_app(pid) {
                        evs.extend(created(self.scan_app(pid)));
                    }
                }
                evs
            }
        }
    }

    fn screen_frames() -> (CGRect, CGRect) {
        use objc2::{class, msg_send, runtime::AnyObject};
        unsafe {
            let screens: *mut AnyObject = msg_send![class!(NSScreen), screens];
            let main: *mut AnyObject = msg_send![screens, firstObject];
            if main.is_null() {
                return (CGRect::default(), CGRect::default());
            }
            let frame: CGRect = msg_send![main, frame];
            let visible: CGRect = msg_send![main, visibleFrame];
            (frame, visible)
        }
    }

    fn place(&self, el: AXUIElementRef, x: f64, y: f64, w: Option<(f64, f64)>) {
        unsafe {
            let p = CGPoint { x, y };
            let pv = AXValueCreate(kAXValueCGPointType, &p as *const _ as *const c_void);
            ax_set(el, "AXPosition", pv);
            CFRelease(pv);
            if let Some((width, height)) = w {
                let s = CGSize { width, height };
                let sv = AXValueCreate(kAXValueCGSizeType, &s as *const _ as *const c_void);
                ax_set(el, "AXSize", sv);
                CFRelease(sv);
            }
        }
    }
}

impl Drop for AxBackend {
    fn drop(&mut self) {
        let area = self.work_area();
        for id in self.parked.drain().collect::<Vec<_>>() {
            if let Some(w) = self.windows.get(&id) {
                self.place(w.element, area.x as f64 + 40.0, area.y as f64 + 40.0, None);
            }
        }
        for id in self.windows.keys().copied().collect::<Vec<_>>() {
            self.forget_window(id);
        }
        for pid in self.apps.keys().copied().collect::<Vec<_>>() {
            self.untrack_app(pid);
        }
    }
}

impl Backend for AxBackend {
    fn work_area(&self) -> Rect {
        let (frame, vis) = Self::screen_frames();
        // AppKit uses a bottom-left origin; AX uses top-left. Flip against the primary screen.
        let top = frame.size.height - (vis.origin.y + vis.size.height);
        Rect::new(
            vis.origin.x as i32,
            top as i32,
            vis.size.width as i32,
            vis.size.height as i32,
        )
    }

    fn existing_windows(&self) -> Vec<WindowId> {
        // Discovery mutates tracking state, which `&self` can't; `run` does a full rescan on
        // its first tick, and `set_float_rules` scans eagerly so startup tiling is immediate.
        self.windows.keys().copied().map(WindowId).collect()
    }

    fn set_float_rules(&mut self, rules: &[String]) {
        self.float_rules = rules.to_vec();
        let pids: Vec<i32> = self.apps.keys().copied().collect();
        for pid in pids {
            self.scan_app(pid);
        }
    }

    fn apply(&mut self, action: &Action) {
        let get = |w: &WindowId| self.windows.get(&w.0).map(|w| (w.pid, w.element));
        match action {
            Action::Place(w, r) => {
                if let Some((_, el)) = get(w) {
                    self.parked.remove(&w.0);
                    self.place(el, r.x as f64, r.y as f64, Some((r.w as f64, r.h as f64)));
                }
            }
            Action::Hide(w) => {
                if let Some((_, el)) = get(w) {
                    let (frame, _) = Self::screen_frames();
                    // macOS keeps at least a sliver on screen; tuck it into the corner.
                    self.place(el, frame.size.width - 1.0, frame.size.height - 1.0, None);
                    self.parked.insert(w.0);
                }
            }
            Action::Show(_) => {} // the Place that follows brings it back
            Action::Focus(w) => {
                if let Some((pid, el)) = get(w) {
                    unsafe {
                        ax_set(el, "AXMain", kCFBooleanTrue);
                        let raise = CfStr::new("AXRaise");
                        AXUIElementPerformAction(el, raise.0);
                        if let Some(app) = self.apps.get(&pid) {
                            ax_set(app.element, "AXFrontmost", kCFBooleanTrue);
                        }
                    }
                }
            }
            Action::Close(w) => {
                if let Some((_, el)) = get(w) {
                    if let Some(button) = ax_copy(el, "AXCloseButton") {
                        let press = CfStr::new("AXPress");
                        unsafe {
                            AXUIElementPerformAction(button, press.0);
                            CFRelease(button);
                        }
                    }
                }
            }
            Action::Quit => unsafe { CFRunLoopStop(CFRunLoopGetMain()) },
        }
    }

    fn waker(&self) -> Waker {
        let src = SendPtr(self.wake_source);
        let rl = SendPtr(unsafe { CFRunLoopGetMain() });
        std::sync::Arc::new(move || unsafe {
            CFRunLoopSourceSignal(src.get());
            CFRunLoopWakeUp(rl.get());
        })
    }

    fn run(
        &mut self,
        bindings: &[(Hotkey, Vec<Command>)],
        handler: &mut Handler<'_>,
    ) -> Result<(), String> {
        if !self.trusted {
            return Err("Accessibility permission is required. Allow this program (or your terminal) under \
                        System Settings > Privacy & Security > Accessibility, then start it again."
                .into());
        }
        self.chords = bindings
            .iter()
            .filter_map(|(h, c)| match key_code(&h.key) {
                Some(k) => Some((k, chord_flags(h), c.clone())),
                None => {
                    eprintln!("warning: unsupported key `{}`, binding skipped", h.key);
                    None
                }
            })
            .collect();

        unsafe {
            let rl = CFRunLoopGetMain();
            self.tap = CGEventTapCreate(
                kCGSessionEventTap,
                kCGHeadInsertEventTap,
                kCGEventTapOptionDefault,
                1 << kCGEventKeyDown,
                tap_callback,
                ptr::null_mut(),
            );
            if self.tap.is_null() {
                return Err(
                    "could not create the keyboard event tap (check Accessibility permission)"
                        .into(),
                );
            }
            let tap_src = CFMachPortCreateRunLoopSource(ptr::null(), self.tap, 0);
            CFRunLoopAddSource(rl, tap_src, kCFRunLoopDefaultMode);
            CFRunLoopAddSource(rl, self.wake_source, kCFRunLoopDefaultMode);

            let mut tctx = CFRunLoopTimerContext {
                version: 0,
                info: ptr::null_mut(),
                retain: ptr::null(),
                release: ptr::null(),
                copy_description: ptr::null(),
            };
            let timer = CFRunLoopTimerCreate(
                ptr::null(),
                CFAbsoluteTimeGetCurrent() + 1.0,
                1.0,
                0,
                0,
                timer_callback,
                &mut tctx,
            );
            CFRunLoopAddTimer(rl, timer, kCFRunLoopDefaultMode);

            let mut ctx = Ctx {
                backend: self,
                handler,
            };
            CTX.with(|c| c.set(&mut ctx as *mut Ctx as *mut c_void));
            CFRunLoopRun();
            CTX.with(|c| c.set(ptr::null_mut()));

            CFRunLoopTimerInvalidate(timer);
            CFRelease(timer);
            CFRunLoopRemoveSource(rl, tap_src, kCFRunLoopDefaultMode);
            CFRunLoopRemoveSource(rl, self.wake_source, kCFRunLoopDefaultMode);
            CFRelease(tap_src);
            CFMachPortInvalidate(self.tap);
            CFRelease(self.tap);
            self.tap = ptr::null();
        }
        Ok(())
    }
}
