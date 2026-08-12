//! The gallery's view tabs, and the shared state behind them.
//!
//! "Son of the day" is a tab here rather than a `Sort` variant. It is not an
//! ordering -- it selects a single featured son from a different query
//! (`db::son_of_the_day`) -- so folding it into `Sort` would have meant a fake
//! ordering in the enum, the wire format and the SQL. `GalleryView` keeps that
//! distinction in the UI layer, where it belongs, and leaves `Sort` untouched.

use leptos::prelude::*;

use crate::models::Sort;

/// What the gallery is currently showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GalleryView {
    /// The full gallery in some order.
    Sort(Sort),
    /// Just today's featured son.
    SonOfDay,
}

impl Default for GalleryView {
    fn default() -> Self {
        Self::Sort(Sort::default())
    }
}

impl GalleryView {
    /// The ordering to query with. `SonOfDay` has no ordering of its own, so it
    /// reports the default -- callers switch on the view before using this.
    pub fn sort(&self) -> Sort {
        match self {
            Self::Sort(s) => *s,
            Self::SonOfDay => Sort::default(),
        }
    }
}

/// Shared gallery view. Provided once by `App` so the tab strip and `Gallery`
/// read one source of truth.
#[derive(Clone, Copy)]
pub struct SortCtx {
    pub view: ReadSignal<GalleryView>,
    pub set_view: WriteSignal<GalleryView>,
}

/// Reads the view context. Panics only if used outside `App`, which is a wiring
/// mistake rather than a runtime condition.
pub fn use_sort() -> SortCtx {
    use_context::<SortCtx>().expect("SortCtx provided by App")
}

/// The view tabs. Labels are display-only -- `Sort`, its `as_str` wire values
/// and the server function signatures are unchanged.
#[component]
pub fn SortChips(#[prop(optional, into)] class: Option<String>) -> impl IntoView {
    let SortCtx { view, set_view } = use_sort();
    let cls = format!("flex min-w-0 items-center gap-2 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]{{display:none}} {}", class.unwrap_or_default());

    // (view, label, accessible name)
    //
    // "Today", not "Son of the day". That one label was as wide as the other
    // three put together, which is what pushed the strip past the edge of every
    // phone and left a chip sliced in half at 320-360px. The full wording still
    // reaches a screen reader through the accessible name below, where length
    // costs nothing -- so nothing is lost by not also spelling it out in a pill.
    // With it shortened the four fit on a 320px screen without scrolling at all.
    let options = [
        (GalleryView::Sort(Sort::Newest), "New", "Sort by newest"),
        (
            GalleryView::Sort(Sort::MostLiked),
            "Cried",
            "Sort by most cried over",
        ),
        (GalleryView::Sort(Sort::Az), "A–Z", "Sort A to Z"),
        (GalleryView::SonOfDay, "Today", "Show son of the day"),
    ];

    view! {
        <div class=cls role="group" aria-label="View">
            {options
                .into_iter()
                .map(|(target, label, aria)| {
                    view! {
                        <button
                            // 36px on touch, 32px with a mouse. Was 44px, which
                            // is Apple's recommended target but not a
                            // requirement -- WCAG 2.2's Target Size (Minimum)
                            // is 24x24 CSS px, so 36 clears it with room. At 44
                            // these were tall rounded lozenges stacked directly
                            // under a 56px bar, and the two bands together ate
                            // the top of a phone screen before a single son
                            // appeared.
                            class="chip min-h-9 lg:min-h-8"
                            class:is-active=move || view.get() == target
                            aria-label=aria
                            // Announced as pressed, so selection is not carried
                            // by colour alone.
                            aria-pressed=move || (view.get() == target).to_string()
                            on:click=move |_| set_view.set(target)
                        >
                            {label}
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}
