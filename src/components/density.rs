//! Grid density presets. Not persisted across visits (an in-memory signal,
//! reset to `Cozy` on every page load) -- keeping it simple avoids the
//! hydration-mismatch complexity of reading `localStorage` before first
//! paint, and a density toggle is a light, low-stakes preference that isn't
//! worth that cost.

use leptos::prelude::*;

use crate::components::icon::{Ico, LuGrid2x2, LuLayoutGrid};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Density {
    Compact,
    #[default]
    Cozy,
}

impl Density {
    pub fn grid_class(&self) -> &'static str {
        match self {
            Density::Compact => {
                "grid grid-cols-3 gap-2 sm:grid-cols-4 xl:grid-cols-5 min-[1900px]:grid-cols-8"
            }
            Density::Cozy => {
                "grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-4 min-[1600px]:grid-cols-5 min-[1900px]:grid-cols-6"
            }
        }
    }
}

/// Whole class strings per state, which is the rule everywhere in this codebase:
/// the Tailwind scanner reads these `.rs` files literally and never sees a class
/// assembled from fragments at runtime.
fn seg_class(active: bool) -> &'static str {
    if active {
        "inline-flex h-8 w-8 items-center justify-center rounded-full bg-surface-hover text-accent transition-colors"
    } else {
        "inline-flex h-8 w-8 items-center justify-center rounded-full text-ink-3 transition-colors hover:text-ink"
    }
}

/// The view-mode switch.
///
/// Was three `.icon-btn`s carrying `class:is-active`, which styled nothing:
/// `is-active` has a rule for `.chip` and no rule for `.icon-btn`, so the
/// selected mode looked exactly like the two unselected ones and the control
/// silently did nothing visible. It also had no `aria-pressed`, so the state was
/// unavailable to a screen reader as well as to the eye. Both are fixed here,
/// and the three loose buttons are now one segmented control in a single
/// enclosure so it reads as a switch rather than as scattered icons.
///
/// Masonry is gone with them. It existed to give each son its true aspect ratio
/// in ragged columns, and `storage::square` now crops every upload to a square,
/// so it had become a slower way to draw the same uniform grid.
#[component]
pub fn DensityToggle(
    density: ReadSignal<Density>,
    set_density: WriteSignal<Density>,
) -> impl IntoView {
    view! {
        <div
            class="ml-auto hidden flex-none items-center gap-0.5 rounded-full border border-line bg-surface p-0.5 min-[700px]:flex"
            role="group"
            aria-label="View mode"
        >
            <button
                class=move || seg_class(density.get() == Density::Compact)
                aria-pressed=move || (density.get() == Density::Compact).to_string()
                on:click=move |_| set_density.set(Density::Compact)
                title="Compact grid"
                aria-label="Compact grid"
            >
                <Ico icon=LuLayoutGrid size=16/>
            </button>
            <button
                class=move || seg_class(density.get() == Density::Cozy)
                aria-pressed=move || (density.get() == Density::Cozy).to_string()
                on:click=move |_| set_density.set(Density::Cozy)
                title="Grid"
                aria-label="Grid"
            >
                <Ico icon=LuGrid2x2 size=16/>
            </button>
        </div>
    }
}

/// A grid of shimmering placeholder tiles, matching the real grid's shape --
/// replaces a bare "loading…" line so the layout doesn't jump once real
/// cards arrive. Square, and with no caption bar, because that is what a card
/// is now.
#[component]
pub fn GridSkeleton(#[prop(default = 8)] count: usize) -> impl IntoView {
    view! {
        <div class="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-4" aria-hidden="true">
            {(0..count)
                .map(|_| {
                    view! {
                        <div class="aspect-square animate-pulse overflow-hidden rounded-lg border border-line bg-surface-raised"></div>
                    }
                })
                .collect_view()}
        </div>
    }
}
