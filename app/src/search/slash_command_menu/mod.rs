pub mod fuzzy_match;
pub mod omp_commands;
pub mod static_commands;
#[cfg(test)]
mod fuzzy_match_tests;

pub use static_commands::{SlashCommandId, StaticCommand};
