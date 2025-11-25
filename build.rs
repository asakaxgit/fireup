fn main() {
    // Ensure protoc is available via vendored binary
    let protoc_path = protoc_bin_vendored::protoc_bin_path().expect("protoc not found");
    std::env::set_var("PROTOC", protoc_path);

    let mut config = prost_build::Config::new();
    config
        .compile_protos(
            &[
                "proto/google/datastore/v1/entity.proto",
                "proto/google/datastore/v1/query.proto",
                "proto/google/datastore/admin/v1/admin.proto",
            ],
            &["proto"],
        )
        .expect("failed to compile protobufs");
}


