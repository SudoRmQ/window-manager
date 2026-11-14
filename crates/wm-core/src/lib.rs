//! Platform-independent core, modelled on i3's internals.
//!
//! - [`tree`]: the container tree (root > outputs > workspaces > split containers > windows),
//!   with per-container layouts, size percentages and focus stacks, and the operations i3
//!   performs on it (attach, split, move, focus by direction, flatten, workspaces).
//! - [`command`]: parser for i3's command language (`focus left; split v`).
//! - [`manager`]: turns OS events and commands into window placements for a backend.
//! - [`ipc`]: i3's IPC wire protocol (`i3-ipc` magic + length + type) and JSON replies.
//!
//! Nothing here touches an OS API, so it builds and tests anywhere.

pub mod command;
pub mod config;
pub mod geometry;
pub mod ipc;
pub mod manager;
pub mod tree;

/// Opaque window handle. Backends map it to an HWND (Windows) or CGWindowID (macOS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u64);
