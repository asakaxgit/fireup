// LevelDB parser module for Firestore backup files
pub mod base64;
pub mod crc32c;
pub mod entity_proto;
pub mod parser;
pub mod protobuf_entity;
pub mod validator;

#[cfg(test)]
mod tests;

pub use parser::LevelDBParser;
pub use validator::{
    BackupValidatorImpl, ValidationResult
};