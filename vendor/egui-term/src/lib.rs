mod backend;
mod bindings;
mod font;
mod theme;
mod types;
mod view;

pub use backend::settings::BackendSettings;
pub use backend::{BackendCommand, PtyEvent, TerminalBackend, TerminalMode};
pub use alacritty_terminal::event::Event as TerminalEvent;
pub use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor as AnsiNamedColor, Rgb as AnsiRgb};
pub use bindings::{Binding, BindingAction, InputKind, KeyboardBinding};
pub use font::{FontSettings, TerminalFont};
pub use theme::{ColorPalette, TerminalTheme};
pub use view::TerminalView;
