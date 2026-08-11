use leptos::prelude::*;
use leptos_meta::{Link, Meta, Title};
use leptos_router::hooks::use_params_map;

use crate::api::sons_by_tag;
use crate::components::card::SonCard;
use crate::components::density::{Density, DensityToggle, GridSkeleton};
use crate::components::empty::{EmptyState, ErrorState};
use crate::components::icon::LuImage;
use crate::components::infinite_scroll::ScrollSentinel;
use crate::models::Son;
use crate::seo::absolute;

/// `/tag/:slug` — the gallery filtered to one tag. Its own route rather than
/// a gallery sort mode, since a tag filter and a sort order are genuinely
/// different axes and conflating them would make the URL scheme ambiguous.
#[component]
pub fn TagPage() -> impl IntoView {
    let params = use_params_map();
    let slug = move || params.read().get("slug").unwrap_or_default();

    let first = Resource::new_blocking(slug, |slug| async move { sons_by_tag(slug, None).await });

    let (extra, set_extra) = signal(Vec::<Son>::new());
    let (cursor, set_cursor) = signal(Option::<String>::None);
    let (exhausted, set_exhausted) = signal(false);

    // Seeds cursor/exhausted once the first page resolves. An Effect, not a
    // side effect inside the view-producing closure below -- see the
    // matching comment in gallery.rs for why: a signal write during render
    // can leave the server's rendered HTML and the client's first hydration
    // pass disagreeing about which branch of the `<Show>` below is active,
    // which crashes the whole wasm module on any tag with fewer than
    // `PAGE_SIZE` sons.
    Effect::new(move |_| {
        if let Some(Ok(page)) = first.get() {
            if cursor.get_untracked().is_none() && !exhausted.get_untracked() {
                match page.next_cursor {
                    Some(c) => set_cursor.set(Some(c)),
                    None => set_exhausted.set(true),
                }
            }
        }
    });

    let load_more = Action::new(move |_: &()| {
        let s = slug();
        let from = cursor.get_untracked();
        async move {
            // See gallery.rs: fetching with a None cursor re-requests page ONE
            // and appends it, duplicating every card. Guarded inside the future
            // because Action's closure must return one.
            let Some(from) = from else { return };
            match sons_by_tag(s, Some(from)).await {
                Ok(page) => {
                    let end = page.next_cursor.is_none();
                    set_extra.update(|v| v.extend(page.sons));
                    set_cursor.set(page.next_cursor);
                    set_exhausted.set(end);
                }
                Err(e) => leptos::logging::error!("could not load more tagged sons: {e}"),
            }
        }
    });
    let loading = load_more.pending();
    let (density, set_density) = signal(Density::default());

    view! {
        <Title text=move || format!("#{} — son collection", slug())/>
        <Meta
            name="description"
            content=move || format!("Every son tagged #{} in the son collection.", slug())
        />
        // Not reactive (`Link::href` doesn't accept a closure the way
        // `Meta::content` does) -- harmless here since crawlers always fetch
        // a fresh SSR response per URL rather than navigating client-side.
        <Link rel="canonical" href=absolute(&format!("/tag/{}", slug()))/>

        <h1 class="m-0 mb-4 text-[1.375rem] font-bold tracking-tight lg:mb-6 lg:text-[1.75rem]">"#" {slug}</h1>

        <div class="flex min-w-0 items-center gap-3 pb-4">
            <DensityToggle density=density set_density=set_density/>
        </div>

        <Suspense fallback=|| view! { <GridSkeleton/> }>
            {move || {
                first
                    .get()
                    .map(|res| match res {
                        Err(_) => {
                            view! { <ErrorState message="Something went wrong."/> }.into_any()
                        }
                        Ok(page) => {
                            if page.sons.is_empty() {
                                return view! {
                                    <EmptyState
                                        icon=LuImage
                                        message="Nothing tagged like this yet."
                                        action_href="/"
                                        action_label="Browse all"
                                    />
                                }
                                    .into_any();
                            }

                            view! {
                                <div class=move || density.get().grid_class()>
                                    <For each=move || page.sons.clone() key=|s| s.id.clone() let:son>
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

        <div class="py-8 text-center">
            <Show
                when=move || !exhausted.get()
                fallback=|| ()
            >
                <ScrollSentinel on_visible=move || {
                    if !loading.get_untracked() {
                        load_more.dispatch(());
                    }
                }/>
                <button
                    class="btn"
                    disabled=move || loading.get()
                    on:click=move |_| {
                        load_more.dispatch(());
                    }
                >
                    {move || if loading.get() { "Loading…" } else { "More" }}
                </button>
            </Show>
        </div>
    }
}
