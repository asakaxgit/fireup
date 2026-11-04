fn main() {
    // Ensure protoc is available via vendored binary
    let protoc_path = protoc_bin_vendored::protoc_bin_path().expect("protoc not found");
    std::env::set_var("PROTOC", protoc_path);

    // Message-only codegen for the export/migration path.
    let mut config = prost_build::Config::new();
    config
        .compile_protos(
            &[
                "proto/google/datastore/v1/entity.proto",
                "proto/google/datastore/v1/query.proto",
                "proto/google/datastore/admin/v1/admin.proto",
                "proto/google/type/latlng.proto",
            ],
            &["proto"],
        )
        .expect("failed to compile protobufs");

    // Messages + gRPC service stubs for the `fireup local` Firestore data plane.
    // Client stubs are generated too so integration tests can dial the server
    // without an external SDK.
    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile(&["proto/google/firestore/v1/firestore.proto"], &["proto"])
        .expect("failed to compile Firestore service protos");

    normalize_keyword_module_names();

    println!("cargo:rerun-if-changed=proto");
}

/// prost-build names the generated file after the proto package and escapes Rust
/// keywords in it, so `package google.type` lands in `google.r#type.rs`. Copy
/// those to the unescaped name so `src/protos/mod.rs` can `include!` a stable
/// path regardless of the prost-build version in use.
fn normalize_keyword_module_names() {
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is set"));

    let entries = std::fs::read_dir(&out_dir).expect("read OUT_DIR");
    for entry in entries.filter_map(Result::ok) {
        let file_name = entry.file_name().to_string_lossy().to_string();
        if !file_name.contains("r#") {
            continue;
        }
        let target = out_dir.join(file_name.replace("r#", ""));
        std::fs::copy(entry.path(), &target)
            .unwrap_or_else(|e| panic!("copy {} to {}: {e}", entry.path().display(), target.display()));
    }
}


