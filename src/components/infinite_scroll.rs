//! A near-invisible sentinel placed just before a "load more" button. Once
//! it scrolls into view, `on_visible` fires -- most visitors never need to
//! click the button at all, but it stays for anyone who'd rather not have
//! the page load more without asking (or whose browser can't run the
//! observer for some reason).

use leptos::prelude::*;

#[component]
pub fn ScrollSentinel(on_visible: impl Fn() + Clone + 'static) -> impl IntoView {
    let node: NodeRef<leptos::html::Div> = NodeRef::new();

    #[cfg(feature = "hydrate")]
    {
        use wasm_bindgen::closure::Closure;
        use wasm_bindgen::JsCast;

        Effect::new(move |_| {
            let Some(el) = node.get() else {
                return;
            };
            let cb = on_visible.clone();

            // Fires with every entry the observer is watching, which is
            // always exactly this one sentinel -- checking "did anything
            // intersect" is enough, no need to identify which entry.
            let closure =
                Closure::<dyn FnMut(js_sys::Array)>::new(move |entries: js_sys::Array| {
                    let visible = entries.iter().any(|entry| {
                        entry
                            .dyn_into::<web_sys::IntersectionObserverEntry>()
                            .map(|e| e.is_intersecting())
                            .unwrap_or(false)
                    });
                    if visible {
                        cb();
                    }
                });

            match web_sys::IntersectionObserver::new(closure.as_ref().unchecked_ref()) {
                Ok(observer) => {
                    observer.observe(&el);
                    // The browser's own reference to the observer keeps it
                    // (and this callback) alive for as long as the element
                    // stays observed, which is the page's lifetime here --
                    // `forget` is the standard wasm-bindgen idiom for a
                    // closure meant to outlive the scope that created it.
                    closure.forget();
                }
                Err(e) => {
                    leptos::logging::error!("IntersectionObserver unavailable: {e:?}");
                }
            }
        });
    }

    #[cfg(not(feature = "hydrate"))]
    let _ = on_visible;

    view! { <div class="scroll-sentinel" node_ref=node></div> }
}
