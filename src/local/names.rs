//! Firestore resource-name parsing.
//!
//! Firestore addresses every document by resource name:
//!
//! ```text
//! projects/{project}/databases/{database}/documents/{collection}/{doc}[/{collection}/{doc}]...
//! ```
//!
//! Shape and rules come from the public Firestore API reference and the
//! `googleapis` protos vendored under `proto/`.

use tonic::Status;

/// A parsed document resource name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentName {
    /// `projects/{project}/databases/{database}/documents`
    pub root: String,
    /// Path segments after `documents/`, always an even-length
    /// collection/document alternation.
    pub segments: Vec<String>,
}

impl DocumentName {
    /// Full resource name, i.e. the store key.
    pub fn full(&self) -> String {
        if self.segments.is_empty() {
            self.root.clone()
        } else {
            format!("{}/{}", self.root, self.segments.join("/"))
        }
    }

    /// Resource name of the parent — the enclosing document, or the database
    /// root for a top-level document.
    pub fn parent(&self) -> String {
        let parent_segments = &self.segments[..self.segments.len() - 2];
        if parent_segments.is_empty() {
            self.root.clone()
        } else {
            format!("{}/{}", self.root, parent_segments.join("/"))
        }
    }

    /// The collection this document lives in (the second-to-last segment).
    pub fn collection_id(&self) -> &str {
        &self.segments[self.segments.len() - 2]
    }

    /// The document's own id (the last segment).
    pub fn document_id(&self) -> &str {
        &self.segments[self.segments.len() - 1]
    }
}

/// Split `projects/{p}/databases/{d}/documents[/rest]` into its root and the
/// remaining path. Rejects anything that is not rooted at a database's
/// `documents` collection.
fn split_root(name: &str) -> Result<(String, Vec<String>), Status> {
    let parts: Vec<&str> = name.split('/').collect();
    let rooted = parts.len() >= 5
        && parts[0] == "projects"
        && parts[2] == "databases"
        && parts[4] == "documents"
        && !parts[1].is_empty()
        && !parts[3].is_empty();

    if !rooted {
        return Err(Status::invalid_argument(format!(
            "resource name must look like \
             projects/{{project}}/databases/{{database}}/documents/..., got {name:?}"
        )));
    }

    let root = parts[..5].join("/");
    let rest = parts[5..]
        .iter()
        .map(|segment| segment.to_string())
        .collect();
    Ok((root, rest))
}

/// Parse a document resource name.
///
/// A document path must have an even number of segments: a document always sits
/// inside a collection. An odd count names a collection, which is a different
/// kind of resource and a common client-side mistake worth a clear error.
pub fn parse_document_name(name: &str) -> Result<DocumentName, Status> {
    let (root, segments) = split_root(name)?;

    if segments.is_empty() {
        return Err(Status::invalid_argument(format!(
            "{name:?} names the database root, not a document"
        )));
    }
    if segments.iter().any(String::is_empty) {
        return Err(Status::invalid_argument(format!(
            "{name:?} has an empty path segment"
        )));
    }
    if segments.len() % 2 != 0 {
        return Err(Status::invalid_argument(format!(
            "{name:?} names a collection ({} segments); documents have an even \
             number of path segments",
            segments.len()
        )));
    }

    Ok(DocumentName { root, segments })
}

/// Parse the `parent` of a `CreateDocument`/`ListDocuments` request, which is
/// either the database root or an enclosing *document*.
pub fn parse_parent_name(parent: &str) -> Result<DocumentName, Status> {
    let (root, segments) = split_root(parent)?;

    if segments.iter().any(String::is_empty) {
        return Err(Status::invalid_argument(format!(
            "{parent:?} has an empty path segment"
        )));
    }
    if segments.len() % 2 != 0 {
        return Err(Status::invalid_argument(format!(
            "parent {parent:?} must be the database root or a document, not a collection"
        )));
    }

    Ok(DocumentName { root, segments })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "projects/demo/databases/(default)/documents";

    #[test]
    fn parses_a_top_level_document() {
        let parsed = parse_document_name(&format!("{ROOT}/users/alice")).expect("valid name");
        assert_eq!(parsed.root, ROOT);
        assert_eq!(parsed.collection_id(), "users");
        assert_eq!(parsed.document_id(), "alice");
        assert_eq!(parsed.parent(), ROOT);
        assert_eq!(parsed.full(), format!("{ROOT}/users/alice"));
    }

    #[test]
    fn parses_a_subcollection_document() {
        let parsed = parse_document_name(&format!("{ROOT}/users/alice/posts/p1")).expect("valid");
        assert_eq!(parsed.collection_id(), "posts");
        assert_eq!(parsed.document_id(), "p1");
        assert_eq!(parsed.parent(), format!("{ROOT}/users/alice"));
    }

    #[test]
    fn rejects_a_collection_path_as_a_document() {
        let err = parse_document_name(&format!("{ROOT}/users")).expect_err("odd segment count");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn rejects_an_unrooted_name() {
        let err = parse_document_name("users/alice").expect_err("missing project/database root");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn accepts_the_database_root_as_a_parent() {
        let parsed = parse_parent_name(ROOT).expect("root is a valid parent");
        assert!(parsed.segments.is_empty());
        assert_eq!(parsed.full(), ROOT);
    }
}
