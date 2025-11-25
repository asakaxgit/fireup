pub mod google {
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
}


