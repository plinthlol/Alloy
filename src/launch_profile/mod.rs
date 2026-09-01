// primitives for parsing, merging, and rendering mojang-format launch
// profiles. used by vanilla and every loader install path (forge/neoforge/
// fabric/quilt) — anything that reads mojang-style version JSON.

pub mod model;
pub mod render;
pub mod resolve;
pub mod rules;
pub mod system;
pub mod templates;
