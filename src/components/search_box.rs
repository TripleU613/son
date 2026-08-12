//! The search form, shared by the mobile top bar and the gallery's utility row.
//!
//! A plain GET to /search rather than a JS-driven search-as-you-type: it works
//! before hydration has loaded, and /search renders its results server-side
//! from the query string either way.

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;

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
    // Shows the current query while on /search, so the box you refine in is the
    // box that already holds what you searched for. Read from the URL rather
    // than kept in a signal: /search renders from the query string on the
    // server too, so this way the field is filled before hydration and for
    // anyone who lands on a shared search link.
    let query = use_query_map();
    let current = move || query.read().get("q").unwrap_or_default();

    view! {
        <form method="get" action="/search" class=cls role="search">
            <span class="pointer-events-none absolute left-3 inline-flex text-ink-3">
                <Ico icon=LuSearch size=16/>
            </span>
            // Deliberately not the `.field` primitive any more.
            //
            // `.field` is a form control: bordered, on `bg-surface`, sized to sit
            // in a column of other inputs. In the top bar that made the loudest,
            // brightest box on a phone screen a search field on a gallery with
            // three items in it -- 57% of the bar's width, taller than the logo,
            // and the first thing the eye landed on. This is the same control
            // written as a quiet pill instead: no border, one step off the
            // background, 36px tall.
            //
            // Written as utilities rather than as overrides on top of `.field`,
            // because border, background and radius would then be set twice at
            // equal specificity and settle on stylesheet order -- the exact
            // coin-flip this project deleted its stylesheet to be rid of.
            // `.field` itself is untouched; the upload, report and admin forms
            // still use it and still want the bordered form-control look.
            <input
                type="search"
                name="q"
                placeholder="Search sons…"
                aria-label="Search sons"
                maxlength="100"
                class="w-full rounded-full border-0 bg-surface-raised py-2 pl-9 pr-3.5 text-[0.875rem] text-ink placeholder:text-ink-3"
                // Both: `value` puts it in the server-rendered HTML, `prop:value`
                // keeps it correct across client-side navigation, where the
                // attribute alone would be ignored once the DOM property exists.
                value=current
                prop:value=current
            />
        </form>
    }
}
