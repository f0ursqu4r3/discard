//! Ghostty theme support: parse a Ghostty config or theme file, find a theme
//! by name, and map its colors to Discard's UI colors. No GPUI dependency.

pub mod bundled;
pub mod color;
pub mod file;
pub mod lookup;
pub mod map;
pub mod resolve;
pub mod spec;
mod x11;
