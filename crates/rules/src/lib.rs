#![forbid(unsafe_code)]

mod loader;
mod matcher;
mod registry;
mod schema;

pub use registry::{LoadedRulePack, RuleRegistry};
pub use schema::{
    ProjectMatcher, RuleAction, RuleDefinition, RuleMatcher, RulePack, RuntimeGuardDefinition,
    rule_pack_schema, scan_location_pack_from_toml, scan_location_pack_schema,
};
