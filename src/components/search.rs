use crate::app::SitePreview;
use leptos::prelude::*;
use leptos_meta::{Meta, Title};
use leptos_router::hooks::use_query_map;

use crate::api::search_sons;
use crate::components::card::SonCard;
use crate::components::density::SearchSkeleton;
use crate::components::empty::{EmptyState, ErrorState};
use crate::components::icon::LuSearch;

/// `/search?q=...` — full-text results via `sons_fts` (see the migration).
/// Not paginated: a search box is a "find the one I mean" tool for a gallery
/// this size, not a second browsing surface, so `PAGE_SIZE` results is
/// already generous for that job.
#[component]
pub fn SearchPage() -> impl IntoView {
    let query = use_query_map();
    let q = move || query.read().get("q").unwrap_or_default();

    let results = Resource::new(q, |q| async move {
        if q.trim().is_empty() {
            Ok(vec![])
        } else {
            search_sons(q).await
        }
    });

    view! {
        <Title text=move || {
            let q = q();
            if q.is_empty() { "search — son collection".to_string() } else { format!("\"{q}\" — son collection") }
        }/>
        <SitePreview/>
        // Query-dependent results have no stable content worth a search
        // engine's crawl budget or a snippet -- `follow` still lets a
        // crawler reach every son a result links to.
        <Meta name="robots" content="noindex, follow"/>

        // The echoed query is the one word on this page that is the visitor's
        // own, so it is the one that gets the accent. A free yellow moment that
        // costs no extra copy, in the only place a query echo belongs.
        <h1 class="m-0 mb-4 text-[1.375rem] font-bold tracking-tight lg:mb-6 lg:text-[1.75rem]">
            {move || {
                let q = q();
                if q.is_empty() {
                    view! { "Search" }.into_any()
                } else {
                    view! {
                        "\u{201C}"<span class="text-accent">{q}</span>"\u{201D}"
                    }
                        .into_any()
                }
            }}
        </h1>

        // 8, not 4: a half-height placeholder grid reads as a page that has
        // finished loading and found four things, then jumps when it hasn't.
        <Suspense fallback=|| view! { <SearchSkeleton/> }>
            {move || {
                results
                    .get()
                    .map(|res| match res {
                        Err(_) => {
                            view! { <ErrorState message="Something went wrong."/> }.into_any()
                        }
                        Ok(sons) if sons.is_empty() => {
                            view! {
                                <EmptyState
                                    icon=LuSearch
                                    message="No results."
                                    action_href="/"
                                    action_label="Clear search"
                                />
                            }
                                .into_any()
                        }
                        Ok(sons) => {
                            view! {
                                // Opacity only, never a transform: animating
                                // the grid wrapper's transform promotes it to
                                // its own layer and can force paint of the very
                                // subtrees the tiles' `content-visibility:auto`
                                // is there to skip.
                                <div class="grid animate-fade-in grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-4">
                                    <For each=move || sons.clone() key=|s| s.id.clone() let:son>
                                        <SonCard son=son/>
                                    </For>
                                </div>
                            }
                                .into_any()
                        }
                    })
            }}
        </Suspense>
    }
}
