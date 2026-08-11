//! The search form, shared by the mobile top bar and the gallery's utility row.
//!
//! A plain GET to /search rather than a JS-driven search-as-you-type: it works
//! before hydration has loaded, and /search renders its results server-side
//! from the query string either way.

use leptos::prelude::*;

use crate::components::icon::{Ico, LuSearch};

#[component]
pub fn SearchBox(#[prop(optional, into)] extra_class: Option<String>) -> impl IntoView {
    let cls = format!("searchbox {}", extra_class.unwrap_or_default());
    view! {
        <form method="get" action="/search" class=cls role="search">
            <span class="searchbox-icon">
                <Ico icon=LuSearch size=16/>
            </span>
            <input
                type="search"
                name="q"
                placeholder="Search sons…"
                aria-label="Search sons"
                maxlength="100"
            />
        </form>
    }
}
