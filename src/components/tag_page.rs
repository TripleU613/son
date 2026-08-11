use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::use_params_map;

use crate::api::sons_by_tag;
use crate::components::card::SonCard;
use crate::models::Son;

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

    let load_more = Action::new(move |_: &()| {
        let s = slug();
        let from = cursor.get_untracked();
        async move {
            match sons_by_tag(s, from).await {
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

    view! {
        <Title text=move || format!("#{} — son collection", slug())/>

        <section class="hero">
            <h1>"#" {slug}</h1>
        </section>

        <Suspense fallback=|| view! { <div class="grid-skeleton">"gathering sons…"</div> }>
            {move || {
                first
                    .get()
                    .map(|res| match res {
                        Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                        Ok(page) => {
                            if cursor.get_untracked().is_none() && !exhausted.get_untracked() {
                                match &page.next_cursor {
                                    Some(c) => set_cursor.set(Some(c.clone())),
                                    None => set_exhausted.set(true),
                                }
                            }

                            if page.sons.is_empty() {
                                return view! {
                                    <section class="empty">
                                        <h2>"Nothing tagged like this yet."</h2>
                                    </section>
                                }
                                    .into_any();
                            }

                            view! {
                                <div class="grid">
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

        <div class="more">
            <Show
                when=move || !exhausted.get()
                fallback=|| view! { <p class="exhausted">"That's every son with this tag."</p> }
            >
                <button
                    class="btn"
                    disabled=move || loading.get()
                    on:click=move |_| {
                        load_more.dispatch(());
                    }
                >
                    {move || if loading.get() { "loading…" } else { "more sons" }}
                </button>
            </Show>
        </div>
    }
}
