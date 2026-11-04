//! Generated protobuf/gRPC stubs.
//!
//! Everything here is compiled at build time from the public `googleapis`
//! `.proto` files vendored under `proto/` (see
//! `scripts/vendor-googleapis-protos.sh` and `docs/licenses/FIREBASE_GCP_LOCAL.md`).

pub mod google {
    pub mod r#type {
        include!(concat!(env!("OUT_DIR"), "/google.type.rs"));
    }
    pub mod api {
        include!(concat!(env!("OUT_DIR"), "/google.api.rs"));
    }
    pub mod rpc {
        include!(concat!(env!("OUT_DIR"), "/google.rpc.rs"));
    }
    pub mod datastore {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/google.datastore.v1.rs"));
        }
        pub mod admin {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/google.datastore.admin.v1.rs"));
            }
        }
    }
    pub mod firestore {
        pub mod v1 {
            include!(concat!(env!("OUT_DIR"), "/google.firestore.v1.rs"));
        }
    }
}
