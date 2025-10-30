// Schema analyzer module
pub mod analyzer;
pub mod constraint_analyzer;
pub mod constraint_generator;
pub mod ddl_generator;
pub mod ddl_output;
pub mod index_generator;
pub mod normalizer;
pub mod type_conflict_resolver;

#[cfg(test)]
mod tests;

pub use analyzer::*;
pub use constraint_generator::*;
pub use ddl_generator::*;
pub use index_generator::*;
pub use normalizer::*;
