//! Platform-independent core: geometry, layouts, workspaces, commands, config.
//! Nothing in this crate touches an OS API, so it builds and tests anywhere.

pub mod command;
pub mod config;
pub mod geometry;
pub mod layout;
pub mod manager;
pub mod workspace;

/// Opaque window handle. Backends map it to an HWND (Windows) or (pid, AXUIElement) (macOS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u64);
