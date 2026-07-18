//! Trusted-base memory-safety side-condition (RFC-0034 §13 clause (c), decomposition split).
//!
//! The `Proven` memory-safety claim for the trusted base rests on the crate-level
//! `#![forbid(unsafe_code)]` attribute. In the monorepo this side-condition is checked by
//! `mycelium-cert`'s conformance suite reading every trusted-base crate's source; in the
//! component-repo split each repo checks the sources it carries (course-correction Phase B,
//! 2026-07-18 — see the runtime repo's `crates/mycelium-cert/tests/conformance.rs` delegation
//! note). If a future edit removed the `forbid`, this fails loudly — the never-silent guard
//! (VR-5/G2).

#[test]
fn trusted_base_forbids_unsafe() {
    let crates_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/mycelium-core has a parent (crates/)");
    let krate = "mycelium-core";
    let lib = crates_dir.join(krate).join("src/lib.rs");
    let src =
        std::fs::read_to_string(&lib).unwrap_or_else(|e| panic!("read {}: {e}", lib.display()));
    assert!(
        src.contains("#![forbid(unsafe_code)]"),
        "{krate}/src/lib.rs must carry `#![forbid(unsafe_code)]` — the checked basis for the \
         RFC-0034 §3.3 memory-safety clause (Proven). Its removal would un-ground the claim."
    );
}
