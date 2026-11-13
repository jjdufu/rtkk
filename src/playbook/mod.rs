//! Playbook data model, `YAML` parser/compiler, dynamic variable injection, and execution engine.

pub(crate) mod engine;
pub(crate) mod etcd;
pub(crate) mod helpers;
pub(crate) mod model;
pub(crate) mod parser;

pub use engine::ExecutionEngine;
pub use etcd::{inject_dynamic_inventory_vars, inject_runtime_vars};
pub use helpers::eval_condition;
pub use model::{Play, Playbook};
pub use parser::compile_playbook;
