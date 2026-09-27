use wasm_bindgen::prelude::wasm_bindgen;

thread_local! {
    static LAST_PANIC: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Records every panic message before the wasm trap, chaining to the previous hook.
#[wasm_bindgen(start)]
pub fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let text = info.to_string();
            LAST_PANIC.with(|slot| *slot.borrow_mut() = Some(text));
            previous(info);
        }));
    });
}

/// The last panic this thread saw; the engine state behind it is unusable.
#[wasm_bindgen]
pub fn last_panic() -> Option<String> {
    LAST_PANIC.with(|slot| slot.borrow().clone())
}
