//! `openatat-ui` — gpui-ce process for Settings, annotation studio, history,
//! and first-run.
//!
//! P0 is a placeholder on purpose:
//! - Idle `openatatd` must not hold a GPU / gpui window.
//! - This binary is spawned on demand and must quit when idle.
//! - gpui-ce 0.3 can do LayerShell / PopUp / Transparent / `focus: false`,
//!   but that is **not** a nonactivating panel. The `@@` overlay stays in
//!   `openatatd` (NSPanel / WS_EX_NOACTIVATE / native layer-shell).
//!
//! P1 will add a `gpui-ce` dependency and an empty Settings window here.
//! We do not take that dependency in P0: gpui-ce pulls a GPU stack that
//! this crate must not force onto `cargo test` of the applet.

fn main() {
    println!(
        "openatat-ui P0 placeholder\n\
         \n\
         This process is for Settings, studio/annotation, history, and first-run.\n\
         Spawn on demand. Quit when idle. Zero GPU windows at applet idle.\n\
         \n\
         Overlay, Orb, trigger, insert, and capture live in openatatd.\n\
         gpui-ce 0.3 LayerShell/PopUp is not a nonactivating panel.\n\
         See SPEC.md (process split) and README.md.\n"
    );
}
