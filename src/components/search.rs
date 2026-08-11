use leptos::prelude::*;
use leptos_meta::{Meta, Title};
use leptos_router::hooks::use_query_map;

use crate::api::search_sons;
use crate::components::card::SonCard;
use crate::components::density::GridSkeleton;
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
        // Query-dependent results have no stable content worth a search
        // engine's crawl budget or a snippet -- `follow` still lets a
        // crawler reach every son a result links to.
        <Meta name="robots" content="noindex, follow"/>

        <h1 class="m-0 mb-4 text-[1.375rem] font-bold tracking-tight lg:mb-6 lg:text-[1.75rem]">
            {move || {
                let q = q();
                if q.is_empty() { "Search".to_string() } else { format!("\u{201C}{q}\u{201D}") }
            }}
        </h1>

        <Suspense fallback=|| view! { <GridSkeleton count=4/> }>
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
                                <div class="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-4">
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
