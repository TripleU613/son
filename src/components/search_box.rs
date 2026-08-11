//! The search form, shared by the mobile top bar and the gallery's utility row.
//!
//! A plain GET to /search rather than a JS-driven search-as-you-type: it works
//! before hydration has loaded, and /search renders its results server-side
//! from the query string either way.

use leptos::prelude::*;

use crate::components::icon::{Ico, LuSearch};

#[component]
pub fn SearchBox(
    /// Reactive so the header can toggle the expanded state; `Signal<String>`
    /// rather than `Option<String>` because the class now changes at runtime.
    #[prop(optional, into)]
    extra_class: Signal<String>,
) -> impl IntoView {
    // `flex-1` on mobile so the field fills the top bar, `lg:flex-none` on
    // desktop where its width comes from the caller's expanded/collapsed class.
    let cls = move || {
        format!(
            "relative flex min-w-0 flex-1 items-center lg:flex-none {}",
            extra_class.get()
        )
    };
    view! {
        <form method="get" action="/search" class=cls role="search">
            <span class="pointer-events-none absolute left-2.5 inline-flex text-ink-3">
                <Ico icon=LuSearch size=16/>
            </span>
            <input
                type="search"
                name="q"
                placeholder="Search sons…"
                aria-label="Search sons"
                maxlength="100"
                class="field pl-8"
            />
        </form>
    }
}
