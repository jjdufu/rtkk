//! Inventory data models, group dependency graph construction, and host variable resolution.

mod graph;
mod model;
mod resolver;

pub use model::{FlatInventory, GroupConfig, Inventory};
pub use resolver::resolve_inventory;
