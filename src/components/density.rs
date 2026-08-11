//! Grid density presets. Not persisted across visits (an in-memory signal,
//! reset to `Cozy` on every page load) -- keeping it simple avoids the
//! hydration-mismatch complexity of reading `localStorage` before first
//! paint, and a density toggle is a light, low-stakes preference that isn't
//! worth that cost.

use leptos::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Density {
    Compact,
    #[default]
    Cozy,
    List,
}

impl Density {
    pub fn grid_class(&self) -> &'static str {
        match self {
            Density::Compact => "grid grid--compact",
            Density::Cozy => "grid grid--cozy",
            Density::List => "grid grid--list",
        }
    }
}

#[component]
pub fn DensityToggle(
    density: ReadSignal<Density>,
    set_density: WriteSignal<Density>,
) -> impl IntoView {
    view! {
        <div class="density-toggle" role="group" aria-label="grid density">
            <button
                class:active=move || density.get() == Density::Compact
                on:click=move |_| set_density.set(Density::Compact)
                title="compact grid"
                aria-label="compact grid"
            >
                "▦"
            </button>
            <button
                class:active=move || density.get() == Density::Cozy
                on:click=move |_| set_density.set(Density::Cozy)
                title="cozy grid"
                aria-label="cozy grid"
            >
                "▤"
            </button>
            <button
                class:active=move || density.get() == Density::List
                on:click=move |_| set_density.set(Density::List)
                title="list view"
                aria-label="list view"
            >
                "☰"
            </button>
        </div>
    }
}

/// A grid of shimmering placeholder cards, matching the real grid's shape --
/// replaces a bare "loading…" line so the layout doesn't jump once real
/// cards arrive.
#[component]
pub fn GridSkeleton(#[prop(default = 8)] count: usize) -> impl IntoView {
    view! {
        <div class="grid grid--cozy" aria-hidden="true">
            {(0..count)
                .map(|_| {
                    view! {
                        <div class="skeleton-card">
                            <div class="skeleton-thumb"></div>
                            <div class="skeleton-line"></div>
                        </div>
                    }
                })
                .collect_view()}
        </div>
    }
}
