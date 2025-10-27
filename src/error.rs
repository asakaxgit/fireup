use thiserror::Error;

/// Main error type for the Fireup system
#[derive(Error, Debug)]
pub enum FireupError {
    #[error("Parse error: {message}")]
    Parse { message: String },
    
    #[error("Schema analysis error: {message}")]
    Schema { message: String },
    
    #[error("Import error: {message}")]
    Import { message: String },
    
    #[error("System error: {message}")]
    System { message: String },
    
    #[error("Database error: {0}")]
    Database(#[from] tokio_postgres::Error),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl FireupError {
    pub fn parse(message: impl Into<String>) -> Self {
        Self::Parse { message: message.into() }
    }
    
    pub fn schema(message: impl Into<String>) -> Self {
        Self::Schema { message: message.into() }
    }
    
    pub fn import(message: impl Into<String>) -> Self {
        Self::Import { message: message.into() }
    }
    
    pub fn system(message: impl Into<String>) -> Self {
        Self::System { message: message.into() }
    }
}