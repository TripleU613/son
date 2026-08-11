//! Grid density presets. Not persisted across visits (an in-memory signal,
//! reset to `Cozy` on every page load) -- keeping it simple avoids the
//! hydration-mismatch complexity of reading `localStorage` before first
//! paint, and a density toggle is a light, low-stakes preference that isn't
//! worth that cost.

use leptos::prelude::*;

use crate::components::icon::{Ico, LuGalleryThumbnails, LuGrid2x2, LuLayoutGrid};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Density {
    Compact,
    #[default]
    Cozy,
    /// Masonry: uneven column runs, each card at its own height. Replaced the
    /// old single-column list view, which was a row of 84px thumbnails and
    /// showed less of each son than any of the grids did.
    Masonry,
}

impl Density {
    pub fn grid_class(&self) -> &'static str {
        match self {
            Density::Compact => "grid grid-cols-3 gap-2 sm:grid-cols-4 xl:grid-cols-5 min-[1900px]:grid-cols-8",
            Density::Cozy => "grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-4 min-[1600px]:grid-cols-5 min-[1900px]:grid-cols-6",
            // CSS multi-column, not grid: real masonry wants columns that run
            // to different lengths, which `columns` gives natively with
            // `break-inside: avoid`. Cards drop the uniform square crop here and
            // use each son's own ratio via the `--son-ratio` custom property the
            // card publishes -- the same mechanism `MoreSons` uses.
            Density::Masonry => "columns-2 gap-3 sm:columns-3 xl:columns-4 min-[1600px]:columns-5 [&_.card]:mb-3 [&_.card]:break-inside-avoid [&_.card-frame]:aspect-[var(--son-ratio,4/5)]",
        }
    }
}

#[component]
pub fn DensityToggle(
    density: ReadSignal<Density>,
    set_density: WriteSignal<Density>,
) -> impl IntoView {
    view! {
        <div class="ml-auto hidden flex-none items-center gap-1 min-[700px]:flex" role="group" aria-label="View mode">
            <button
                class="icon-btn rounded-full"
                class:is-active=move || density.get() == Density::Compact
                on:click=move |_| set_density.set(Density::Compact)
                title="Compact grid"
                aria-label="Compact grid"
            >
                <Ico icon=LuLayoutGrid size=16/>
            </button>
            <button
                class="icon-btn rounded-full"
                class:is-active=move || density.get() == Density::Cozy
                on:click=move |_| set_density.set(Density::Cozy)
                title="Grid"
                aria-label="Grid"
            >
                <Ico icon=LuGrid2x2 size=16/>
            </button>
            <button
                class="icon-btn rounded-full"
                class:is-active=move || density.get() == Density::Masonry
                on:click=move |_| set_density.set(Density::Masonry)
                title="Masonry"
                aria-label="Masonry"
            >
                <Ico icon=LuGalleryThumbnails size=16/>
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
        <div class="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-4" aria-hidden="true">
            {(0..count)
                .map(|_| {
                    view! {
                        <div class="overflow-hidden rounded-lg border border-line bg-surface">
                            <div class="aspect-[4/5] animate-pulse bg-surface-raised"></div>
                            <div class="m-3 h-3.5 animate-pulse rounded bg-surface-raised"></div>
                        </div>
                    }
                })
                .collect_view()}
        </div>
    }
}
