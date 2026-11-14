use crate::layout::LayoutKind;
use std::str::FromStr;

/// Everything a keybinding (or, later, the IPC socket) can ask the manager to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    FocusNext,
    FocusPrev,
    SwapNext,
    SwapPrev,
    Promote,
    GrowMaster,
    ShrinkMaster,
    IncMasterCount,
    DecMasterCount,
    SetLayout(LayoutKind),
    CycleLayout,
    ToggleFloat,
    Workspace(usize),
    MoveToWorkspace(usize),
    Retile,
    Quit,
}

impl FromStr for Command {
    type Err = String;

    /// Parses strings like `focus-next`, `layout bsp`, `workspace 3`.
    fn from_str(s: &str) -> Result<Self, String> {
        let mut parts = s.split_whitespace();
        let name = parts.next().ok_or("empty command")?;
        let arg = parts.next();
        let num = || -> Result<usize, String> {
            let n: usize = arg.ok_or(format!("`{name}` needs a number"))?
                .parse().map_err(|_| format!("bad number in `{s}`"))?;
            if n == 0 { Err("workspaces are numbered from 1".into()) } else { Ok(n - 1) }
        };
        Ok(match name {
            "focus-next" => Command::FocusNext,
            "focus-prev" => Command::FocusPrev,
            "swap-next" => Command::SwapNext,
            "swap-prev" => Command::SwapPrev,
            "promote" => Command::Promote,
            "grow-master" => Command::GrowMaster,
            "shrink-master" => Command::ShrinkMaster,
            "inc-master" => Command::IncMasterCount,
            "dec-master" => Command::DecMasterCount,
            "cycle-layout" => Command::CycleLayout,
            "toggle-float" => Command::ToggleFloat,
            "retile" => Command::Retile,
            "quit" => Command::Quit,
            "workspace" => Command::Workspace(num()?),
            "move-to-workspace" => Command::MoveToWorkspace(num()?),
            "layout" => Command::SetLayout(match arg {
                Some("bsp") => LayoutKind::Bsp,
                Some("master-stack") => LayoutKind::MasterStack,
                Some("monocle") => LayoutKind::Monocle,
                other => return Err(format!("unknown layout {other:?}")),
            }),
            _ => return Err(format!("unknown command `{name}`")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        assert_eq!("focus-next".parse(), Ok(Command::FocusNext));
        assert_eq!("workspace 2".parse(), Ok(Command::Workspace(1)));
        assert_eq!("layout monocle".parse(), Ok(Command::SetLayout(LayoutKind::Monocle)));
        assert!("workspace 0".parse::<Command>().is_err());
        assert!("explode".parse::<Command>().is_err());
    }
}
