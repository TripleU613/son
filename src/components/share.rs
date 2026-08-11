//! Share a son: the OS share sheet where there is one, copy-to-clipboard where
//! there isn't.
//!
//! One control, not two. On a phone `navigator.share` opens the real share sheet
//! (Messages, WhatsApp, AirDrop), which is what someone actually wants when they
//! find a son worth sending. On a desktop browser that API is mostly absent, so
//! the same button writes the URL to the clipboard instead. Offering both
//! side by side would put a dead button in front of half the visitors.
//!
//! The label and icon follow the capability rather than the viewport, because
//! that is what the button will really do -- deciding by CSS breakpoint would
//! promise a share sheet to a narrow desktop window that has no such thing.

use leptos::prelude::*;

use crate::components::icon::{Ico, LuCheck, LuLink, LuShare2};

/// How long the confirmation stays up after a copy, in milliseconds. Long
/// enough to read, short enough that it doesn't look stuck.
///
/// hydrate-only, like the copy itself: the server build has no clipboard to
/// confirm a write to, and an unused const there is a clippy failure.
#[cfg(feature = "hydrate")]
const CONFIRM_MS: u64 = 1600;

#[component]
pub fn ShareButton(
    /// Absolute page URL to share. Passed in rather than read from
    /// `window.location` so the server render and the client agree, and so it
    /// stays the canonical origin rather than whatever host is in the address
    /// bar (a tailnet IP, a preview domain).
    #[prop(into)]
    url: String,
    /// Used as the share sheet's title; ignored by the clipboard path.
    #[prop(into)]
    title: String,
) -> impl IntoView {
    let (copied, set_copied) = signal(false);

    // Capability, not viewport. Starts false so SSR and the first client render
    // produce identical markup -- flipping it during render is what crashes
    // hydration, so it moves to an effect that only ever runs on the client.
    let (can_share, set_can_share) = signal(false);

    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        let has = js_sys::Reflect::has(
            &web_sys::window().expect("a browser has a window"),
            &wasm_bindgen::JsValue::from_str("navigator"),
        )
        .unwrap_or(false)
            && js_sys::Reflect::has(
                &web_sys::window()
                    .expect("a browser has a window")
                    .navigator(),
                &wasm_bindgen::JsValue::from_str("share"),
            )
            .unwrap_or(false);
        set_can_share.set(has);
    });

    // Both setters are written only from the hydrate-only paths below. The
    // server renders the button inert -- correct, since there is nothing to copy
    // to until wasm is running -- so the server build sees them as unused.
    #[cfg(not(feature = "hydrate"))]
    let _ = (set_can_share, set_copied);

    let on_click = {
        let url = url.clone();
        let title = title.clone();
        move |_| {
            #[cfg(feature = "hydrate")]
            {
                let title = title.clone();
                let window = web_sys::window().expect("a browser has a window");

                // `absolute()` only consults SITE_ORIGIN under the ssr feature,
                // so in the wasm build it hands back exactly what it was given
                // -- and this component runs on the client. Left alone, the
                // button copied "/son/<id>", a path rather than a link, which
                // pastes into a chat as nothing anyone can click. Resolved
                // against the origin actually being viewed, which is also the
                // right answer when browsing over the tailnet.
                let url = {
                    let u = url.clone();
                    if u.starts_with("http://") || u.starts_with("https://") {
                        u
                    } else {
                        let origin = window
                            .location()
                            .origin()
                            .unwrap_or_default()
                            .trim_end_matches('/')
                            .to_string();
                        format!("{origin}{u}")
                    }
                };

                let nav = window.navigator();

                if can_share.get_untracked() {
                    let data = web_sys::ShareData::new();
                    data.set_title(&title);
                    data.set_url(&url);
                    // The returned promise rejects when the sheet is dismissed,
                    // which is a normal user action rather than an error, so it
                    // is deliberately not surfaced or logged.
                    let _ = nav.share_with_data(&data);
                    return;
                }

                // Clipboard writes are async and can be refused outright (no
                // permission, or a non-secure origin). Only confirm once the
                // write has actually resolved -- claiming "Copied" over a failed
                // write would leave someone pasting whatever was there before.
                let promise = nav.clipboard().write_text(&url);
                leptos::task::spawn_local(async move {
                    match wasm_bindgen_futures::JsFuture::from(promise).await {
                        Ok(_) => {
                            set_copied.set(true);
                            set_timeout(
                                move || set_copied.set(false),
                                std::time::Duration::from_millis(CONFIRM_MS),
                            );
                        }
                        Err(_) => leptos::logging::warn!("clipboard write refused"),
                    }
                });
            }
            #[cfg(not(feature = "hydrate"))]
            {
                let _ = (&url, &title);
            }
        }
    };

    let label = move || {
        if copied.get() {
            "Link copied"
        } else if can_share.get() {
            "Share"
        } else {
            "Copy link"
        }
    };

    view! {
        // aria-live so the confirmation is announced, not just seen: the only
        // feedback for a copy is the icon swap, which a screen reader user
        // would otherwise get nothing from.
        <button
            class="icon-btn"
            type="button"
            on:click=on_click
            aria-label=label
            title=label
            aria-live="polite"
        >
            {move || {
                if copied.get() {
                    view! { <span class="text-ok"><Ico icon=LuCheck/></span> }.into_any()
                } else if can_share.get() {
                    view! { <Ico icon=LuShare2/> }.into_any()
                } else {
                    view! { <Ico icon=LuLink/> }.into_any()
                }
            }}
        </button>
    }
}
