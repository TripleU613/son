use crate::app::SitePreview;
use leptos::prelude::*;
use leptos_meta::{Link, Meta, Title};

use crate::api::{list_sons, son_of_the_day};
use crate::components::card::SonCard;
use crate::components::density::{Density, DensityToggle, GridSkeleton};
use crate::components::empty::EmptyState;
use crate::components::icon::LuImage;
use crate::components::infinite_scroll::ScrollSentinel;
use crate::components::more_sons::MoreSons;
use crate::components::sort_chips::{use_sort, GalleryView, SortChips, SortCtx};
use crate::models::{Son, Sort};
use crate::seo::absolute;

const DESCRIPTION: &str = "Every son, collected. Sonion. Capri-Son. Dy-Son. \
Sonflower. If it has a son in it, it belongs here — a free, community-run \
gallery of the son meme, open to anonymous uploads.";

/// The gallery.
///
/// The first page comes from a blocking `Resource` so it renders during SSR — a
/// meme site lives on shared links, and a client-only grid would hand crawlers
/// an empty page. Later pages accumulate in a plain signal and append
/// client-side.
#[component]
pub fn Gallery() -> impl IntoView {
    // The active view comes from app-level context; the tab strip owns writing
    // it, this component only reads.
    let SortCtx { view, .. } = use_sort();
    let sort = Signal::derive(move || view.get().sort());
    let son_of_day_view = Signal::derive(move || view.get() == GalleryView::SonOfDay);

    // Keyed on the ordering, so switching tabs refetches page one from the
    // server rather than re-sorting a partial list in the browser. Not keyed on
    // the view itself: New -> Son of the day -> New would otherwise throw away
    // and re-fetch an identical page.
    let first = Resource::new_blocking(
        move || sort.get(),
        |s| async move { list_sons(None, Some(s.as_str().to_string())).await },
    );

    // Pages 2..n, appended below whatever SSR delivered.
    let (extra, set_extra) = signal(Vec::<Son>::new());
    let (cursor, set_cursor) = signal(Option::<String>::None);
    let (exhausted, set_exhausted) = signal(false);
    // Tracked separately from `exhausted`: an empty gallery is technically
    // exhausted, but rendering "That's every son. For now." directly under
    // "No sons yet." is self-contradictory, and it was the first thing a
    // visitor saw on the freshly-launched site.
    let (is_empty, set_is_empty) = signal(false);

    // Seeds cursor/exhausted once the first page resolves. An Effect, not a
    // side effect inside the view-producing closure below: a signal write
    // during render can leave the server's rendered HTML and the client's
    // first hydration pass disagreeing about which branch of the `<Show>`
    // near the bottom is active, since the server has no equivalent
    // "post-render" phase to run it in and so never sees it at all. Found
    // via a real crash -- any gallery with fewer than `PAGE_SIZE` sons
    // (every brand-new deployment, on day one) hit a hydration panic that
    // took down the whole wasm module. Effects are already client-only by
    // design, so this can't happen here.
    Effect::new(move |_| {
        if let Some(Ok(page)) = first.get() {
            set_is_empty.set(page.sons.is_empty());
            if cursor.get_untracked().is_none() && !exhausted.get_untracked() {
                match page.next_cursor {
                    Some(c) => set_cursor.set(Some(c)),
                    None => set_exhausted.set(true),
                }
            }
        }
    });

    let load_more = Action::new(move |_: &()| {
        let from = cursor.get_untracked();
        let s = sort.get_untracked();
        async move {
            // A None cursor means "nothing more to continue from" -- either
            // page one has not resolved yet, or the gallery is exhausted.
            // Fetching with None re-requests page ONE and appends it, which
            // duplicated every card: changing sort resets `exhausted`, the
            // scroll sentinel re-fires before the new first page lands, and the
            // whole list arrived twice. Page one comes from the blocking
            // Resource and never from here. Guarded inside the future because
            // Action's closure must return one.
            let Some(from) = from else { return };
            match list_sons(Some(from), Some(s.as_str().to_string())).await {
                Ok(page) => {
                    let end = page.next_cursor.is_none();
                    set_extra.update(|v| v.extend(page.sons));
                    set_cursor.set(page.next_cursor);
                    set_exhausted.set(end);
                }
                Err(e) => leptos::logging::error!("could not load more sons: {e}"),
            }
        }
    });

    let loading = load_more.pending();
    let (density, set_density) = signal(Density::default());

    // Switching sort invalidates everything accumulated under the old order.
    // An Effect keyed on `sort`, because the chips that change it now live in
    // the header and no longer have a handler here to hook into. Runs once on
    // mount too, where resetting already-empty accumulators is a no-op.
    Effect::new(move |prev: Option<Sort>| {
        let current = sort.get();
        if prev.is_some_and(|p| p != current) {
            set_extra.set(Vec::new());
            set_cursor.set(None);
            set_exhausted.set(false);
            set_is_empty.set(false);
        }
        current
    });

    view! {
        <Title text="son collection — every son, collected"/>
        <SitePreview/>
        <Meta name="description" content=DESCRIPTION/>
        <Link rel="canonical" href=absolute("/")/>

        // No hero. The gallery is the product, so content starts at the top of
        // the page; the title/tagline/marketing copy the old layout opened with
        // is gone rather than reworded.
        // Tabs and the view-mode toggle share one row: tabs left, modes right.
        // The son-of-the-day banner that used to sit above this is now the last
        // tab instead of a full-width card competing with the grid.
        // The tab strip scrolls sideways when it does not fit (it stops fitting
        // around 360px). Bled out to the viewport edges with matching inner
        // padding so a half-scrolled chip is cut off by the edge of the screen,
        // which reads as "there is more this way"; clipped at the content
        // gutter instead, the same chip just looks broken. Cancelled at the
        // breakpoint where the density toggle joins the row and the strip stops
        // owning the full width.
        <div class="flex min-w-0 items-center gap-3 pb-3 min-[700px]:pb-4">
            <SortChips class="flex-1 -mx-4 px-4 min-[700px]:mx-0 min-[700px]:px-0"/>
            <DensityToggle density=density set_density=set_density/>
        </div>

        <Show when=move || son_of_day_view.get() fallback=|| ()>
            <SonOfTheDay/>
        </Show>

        <Show when=move || !son_of_day_view.get() fallback=|| ()>
        <Suspense fallback=|| view! { <GridSkeleton/> }>
            {move || {
                first
                    .get()
                    .map(|res| match res {
                        Err(e) => {
                            view! { <p class="text-danger">"Could not reach the sons: " {e.to_string()}</p> }
                                .into_any()
                        }
                        Ok(page) => {
                            if page.sons.is_empty() {
                                return view! {
                                    <EmptyState
                                        icon=LuImage
                                        message="No sons yet."
                                        action_href="/upload"
                                        action_label="Contribute"
                                    />
                                }
                                    .into_any();
                            }

                            view! {
                                <div class=move || density.get().grid_class()>
                                    <For
                                        each=move || page.sons.clone()
                                        key=|s| s.id.clone()
                                        let:son
                                    >
                                        <SonCard son=son/>
                                    </For>
                                    <For each=move || extra.get() key=|s| s.id.clone() let:son>
                                        <SonCard son=son/>
                                    </For>
                                </div>
                            }
                                .into_any()
                        }
                    })
            }}
        </Suspense>
        </Show>

        <div
            class="py-6 text-center"
            class:hidden=move || is_empty.get() || son_of_day_view.get()
        >
            <Show
                when=move || !exhausted.get()
                fallback=|| ()
            >
                // The sentinel fetches the next page as it comes into view.
                <ScrollSentinel on_visible=move || {
                    if !loading.get_untracked() {
                        load_more.dispatch(());
                    }
                }/>
                // Status, not a control. The "more sons" button that used to sit
                // here fired the same action the sentinel had already fired by
                // the time anyone could scroll far enough to press it -- so it
                // was a button whose job was always finished before it was
                // reachable. What is actually worth showing is whether a fetch
                // is in flight.
                //
                // `aria-live` because the alternative to a button is that new
                // sons appear silently, which a screen reader would otherwise
                // never mention.
                <p class="m-0 text-[0.8125rem] text-ink-3" aria-live="polite">
                    {move || if loading.get() { "loading more sons…" } else { "" }}
                </p>
            </Show>
        </div>
    }
}

/// The most-liked son from the last 24h (or overall, on a quiet day) —
/// computed on read in `db::son_of_the_day`, not curated by anyone.
#[component]
fn SonOfTheDay() -> impl IntoView {
    let featured = Resource::new(|| (), |_| son_of_the_day());

    view! {
        <Suspense fallback=|| view! { <GridSkeleton count=1/> }>
            {move || {
                match featured.get().and_then(Result::ok).flatten() {
                    // Its own tab now, so it gets the grid's card treatment
                    // rather than a full-width banner shouting above the
                    // gallery. One card, sized like the others.
                    Some(s) => {
                        let id = s.id.clone();
                        view! {
                            <div class="max-w-[320px]">
                                <SonCard son=s/>
                            </div>
                            // Fills the rest of the page rather than leaving one
                            // card alone in the viewport.
                            <MoreSons exclude=id/>
                        }
                            .into_any()
                    }
                    // Nothing featured yet means an empty collection; the same
                    // empty state the gallery uses.
                    None => {
                        view! {
                            <EmptyState
                                icon=LuImage
                                message="No son of the day yet."
                                action_href="/upload"
                                action_label="Contribute"
                            />
                        }
                            .into_any()
                    }
                }
            }}
        </Suspense>
    }
}
