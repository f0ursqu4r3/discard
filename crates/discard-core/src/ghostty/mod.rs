//! Ghostty theme support: parse a Ghostty config or theme file, find a theme
//! by name, and map its colors to Discard's UI colors. No GUI dependency.

pub mod bundled;
pub mod color;
pub mod copy;
pub mod file;
pub mod font;
pub mod live;
pub mod lookup;
pub mod map;
pub mod resolve;
pub mod spec;
mod x11;
