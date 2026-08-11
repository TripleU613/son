use leptos::prelude::*;
use leptos_meta::{Meta, Title};
use leptos_router::hooks::use_query_map;

use crate::api::search_sons;
use crate::components::card::SonCard;

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

        <section class="hero">
            <h1>"search"</h1>
            <p>{move || if q().is_empty() { "Type something in the box above.".to_string() } else { format!("Results for \"{}\"", q()) }}</p>
        </section>

        <Suspense fallback=|| view! { <div class="grid-skeleton">"searching…"</div> }>
            {move || {
                results
                    .get()
                    .map(|res| match res {
                        Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                        Ok(sons) if sons.is_empty() => {
                            view! {
                                <section class="empty">
                                    <h2>"No matches."</h2>
                                    <p>"Try a different word, or check the spelling of the son."</p>
                                </section>
                            }
                                .into_any()
                        }
                        Ok(sons) => {
                            view! {
                                <div class="grid">
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
