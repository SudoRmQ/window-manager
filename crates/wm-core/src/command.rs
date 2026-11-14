//! i3's command language (a practical subset). Commands are separated by `;` or `,`.
//!
//! ```text
//! focus left|right|up|down|parent|child
//! move left|right|up|down
//! move [container|window] to workspace [number] <name>|next|prev|back_and_forth
//! split h|v|horizontal|vertical|toggle
//! layout stacking|tabbed|splith|splitv|default|toggle [split|all]
//! fullscreen [toggle|enable|disable]
//! floating toggle|enable|disable
//! workspace [number] <name>|next|prev|back_and_forth
//! resize grow|shrink width|height [N px] [or N ppt]
//! kill | exit | nop
//! ```

use crate::tree::{Direction, Layout, Orientation, WsTarget};

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Focus(Direction),
    FocusParent,
    FocusChild,
    Move(Direction),
    MoveToWorkspace(WsTarget),
    Split(Layout),
    SplitToggle,
    Layout(Layout),
    LayoutToggleSplit,
    LayoutToggleAll,
    LayoutDefault,
    Fullscreen(Toggle),
    Floating(Toggle),
    Workspace(WsTarget),
    /// Change the focused con's share along `orientation` by `delta` (fraction, e.g. 0.1).
    Resize {
        orientation: Orientation,
        delta: f64,
    },
    Kill,
    Exit,
    Nop,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Toggle {
    Toggle,
    Enable,
    Disable,
}

fn direction(s: &str) -> Option<Direction> {
    Some(match s {
        "left" => Direction::Left,
        "right" => Direction::Right,
        "up" => Direction::Up,
        "down" => Direction::Down,
        _ => return None,
    })
}

fn toggle(s: Option<&str>) -> Result<Toggle, String> {
    match s {
        None | Some("toggle") => Ok(Toggle::Toggle),
        Some("enable") => Ok(Toggle::Enable),
        Some("disable") => Ok(Toggle::Disable),
        Some(o) => Err(format!("expected toggle|enable|disable, got `{o}`")),
    }
}

fn ws_target(words: &[&str]) -> Result<WsTarget, String> {
    let words = match words.first() {
        Some(&"number") => &words[1..],
        _ => words,
    };
    match words {
        [] => Err("missing workspace name".into()),
        ["next"] | ["next_on_output"] => Ok(WsTarget::Next),
        ["prev"] | ["prev_on_output"] => Ok(WsTarget::Prev),
        ["back_and_forth"] => Ok(WsTarget::BackAndForth),
        _ => Ok(WsTarget::Name(
            words.join(" ").trim_matches('"').to_string(),
        )),
    }
}

fn parse_one(input: &str) -> Result<Command, String> {
    let lower = input.to_lowercase();
    let w: Vec<&str> = lower.split_whitespace().collect();
    // Workspace names keep their original case.
    let orig: Vec<&str> = input.split_whitespace().collect();
    let err = || format!("unknown command `{}`", input.trim());
    Ok(match w.as_slice() {
        [] | ["nop", ..] => Command::Nop,
        ["focus", "parent"] => Command::FocusParent,
        ["focus", "child"] => Command::FocusChild,
        ["focus", d] => Command::Focus(direction(d).ok_or_else(err)?),
        ["move", d] | ["move", d, _, "px"] | ["move", d, _] if direction(d).is_some() => {
            Command::Move(direction(d).unwrap())
        }
        ["move", rest @ ..] => {
            let skip = match rest {
                ["container" | "window", "to", "workspace", ..] => 4,
                ["to", "workspace", ..] => 3,
                ["workspace", ..] => 2,
                _ => return Err(err()),
            };
            Command::MoveToWorkspace(ws_target(&orig[skip..])?)
        }
        ["split", "h" | "horizontal"] => Command::Split(Layout::Splith),
        ["split", "v" | "vertical"] => Command::Split(Layout::Splitv),
        ["split", "t" | "toggle"] => Command::SplitToggle,
        ["layout", "stacking" | "stacked"] => Command::Layout(Layout::Stacked),
        ["layout", "tabbed"] => Command::Layout(Layout::Tabbed),
        ["layout", "splith"] => Command::Layout(Layout::Splith),
        ["layout", "splitv"] => Command::Layout(Layout::Splitv),
        ["layout", "default"] => Command::LayoutDefault,
        ["layout", "toggle"] | ["layout", "toggle", "split"] => Command::LayoutToggleSplit,
        ["layout", "toggle", "all"] => Command::LayoutToggleAll,
        ["fullscreen", rest @ ..] => Command::Fullscreen(toggle(rest.first().copied())?),
        ["floating", rest @ ..] => Command::Floating(toggle(rest.first().copied())?),
        ["workspace", ..] => Command::Workspace(ws_target(&orig[1..])?),
        ["resize", how @ ("grow" | "shrink"), dim @ ("width" | "height"), rest @ ..] => {
            // i3 syntax: `resize grow width 10 px or 10 ppt`. ppt is used when given; a bare px
            // amount is converted at 1000 px = 100 ppt (a rough screen width).
            let ppt = rest
                .windows(2)
                .find(|p| p[1] == "ppt")
                .and_then(|p| p[0].parse::<f64>().ok());
            let px = rest.first().and_then(|n| n.parse::<f64>().ok());
            let amount = ppt
                .map(|p| p / 100.0)
                .or(px.map(|p| p / 1000.0))
                .unwrap_or(0.1);
            Command::Resize {
                orientation: if *dim == "width" {
                    Orientation::Horizontal
                } else {
                    Orientation::Vertical
                },
                delta: if *how == "grow" { amount } else { -amount },
            }
        }
        ["kill"] => Command::Kill,
        ["exit"] | ["quit"] => Command::Exit,
        _ => return Err(err()),
    })
}

/// Parse a command line like `split v; layout tabbed`.
pub fn parse(input: &str) -> Result<Vec<Command>, String> {
    input
        .split([';', ','])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_one)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(s: &str) -> Command {
        let mut v = parse(s).unwrap();
        assert_eq!(v.len(), 1);
        v.remove(0)
    }

    #[test]
    fn parses_i3_commands() {
        assert_eq!(one("focus left"), Command::Focus(Direction::Left));
        assert_eq!(one("move up"), Command::Move(Direction::Up));
        assert_eq!(one("split v"), Command::Split(Layout::Splitv));
        assert_eq!(one("layout stacking"), Command::Layout(Layout::Stacked));
        assert_eq!(one("layout toggle split"), Command::LayoutToggleSplit);
        assert_eq!(
            one("fullscreen toggle"),
            Command::Fullscreen(Toggle::Toggle)
        );
        assert_eq!(
            one("workspace number 3"),
            Command::Workspace(WsTarget::Name("3".into()))
        );
        assert_eq!(
            one("workspace Mail Box"),
            Command::Workspace(WsTarget::Name("Mail Box".into()))
        );
        assert_eq!(
            one("move container to workspace back_and_forth"),
            Command::MoveToWorkspace(WsTarget::BackAndForth)
        );
        assert_eq!(
            one("resize grow width 10 px or 10 ppt"),
            Command::Resize {
                orientation: Orientation::Horizontal,
                delta: 0.1
            }
        );
    }

    #[test]
    fn chains() {
        assert_eq!(parse("split h; layout tabbed").unwrap().len(), 2);
    }

    #[test]
    fn rejects_unknown() {
        assert!(parse("fly away").is_err());
        assert!(parse("focus sideways").is_err());
    }
}
