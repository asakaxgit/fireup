// Schema analyzer module
pub mod analyzer;
pub mod ddl_generator;
pub mod normalizer;
pub mod type_conflict_resolver;

#[cfg(test)]
mod tests;

pub use analyzer::*;
pub use ddl_generator::*;
pub use normalizer::*;
