//! An engine panic is an experimental OUTCOME (a violated oracle), not a
//! console footnote: the hook must keep the message where the lab can read it
//! back and render an abort banner.

#[test]
fn a_trapped_panic_is_recoverable_as_text() {
    sim_wasm::install_panic_hook();
    assert!(sim_wasm::last_panic().is_none(), "clean before any trap");

    let trapped = std::panic::catch_unwind(|| panic!("deliberate: T below the floor"));
    assert!(trapped.is_err(), "the helper must really panic");

    let message = sim_wasm::last_panic().expect("the hook recorded the trap");
    assert!(
        message.contains("deliberate: T below the floor"),
        "the payload travels: {message}"
    );
    assert!(
        message.contains("panic_capture.rs"),
        "and so does the location: {message}"
    );
}
