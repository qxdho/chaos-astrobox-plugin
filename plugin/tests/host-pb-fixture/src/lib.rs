//! Test-only host schema/codegen fixture; never linked into the plugin release.
pub mod protocol {
    include!(concat!(env!("OUT_DIR"), "/protocol.rs"));
    include!(concat!(env!("OUT_DIR"), "/protocol.serde.rs"));
}
