use leptos::prelude::*;
use leptos_meta::Title;

use crate::api::{list_sons, son_of_the_day};
use crate::components::card::SonCard;
use crate::models::{Son, Sort};

/// The gallery.
///
/// The first page comes from a blocking `Resource` so it renders during SSR — a
/// meme site lives on shared links, and a client-only grid would hand crawlers
/// an empty page. Later pages accumulate in a plain signal and append
/// client-side.
#[component]
pub fn Gallery() -> impl IntoView {
    let (sort, set_sort) = signal(Sort::Newest);

    // Keyed on `sort`, so flipping the order refetches page one from the server
    // rather than re-sorting a partial list in the browser.
    let first = Resource::new_blocking(
        move || sort.get(),
        |s| async move { list_sons(None, Some(s.as_str().to_string())).await },
    );

    // Pages 2..n, appended below whatever SSR delivered.
    let (extra, set_extra) = signal(Vec::<Son>::new());
    let (cursor, set_cursor) = signal(Option::<String>::None);
    let (exhausted, set_exhausted) = signal(false);

    let load_more = Action::new(move |_: &()| {
        let from = cursor.get_untracked();
        let s = sort.get_untracked();
        async move {
            match list_sons(from, Some(s.as_str().to_string())).await {
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

    // Switching sort invalidates everything accumulated under the old order.
    let choose = move |s: Sort| {
        if sort.get_untracked() != s {
            set_extra.set(Vec::new());
            set_cursor.set(None);
            set_exhausted.set(false);
            set_sort.set(s);
        }
    };

    view! {
        <Title text="son collection — every son, collected"/>

        <section class="hero">
            <h1>"the son collection"</h1>
            <p>"Sonion. Capri-Son. Dy-Son. Sonflower. If it has a son in it, it belongs here."</p>
        </section>

        <SonOfTheDay/>

        <div class="sortbar">
            <button
                class:active=move || sort.get() == Sort::Newest
                on:click=move |_| choose(Sort::Newest)
            >
                "newest"
            </button>
            <button
                class:active=move || sort.get() == Sort::MostLiked
                on:click=move |_| choose(Sort::MostLiked)
            >
                "most cried over"
            </button>
        </div>

        <Suspense fallback=|| view! { <div class="grid-skeleton">"gathering sons…"</div> }>
            {move || {
                first
                    .get()
                    .map(|res| match res {
                        Err(e) => {
                            view! { <p class="error">"Could not reach the sons: " {e.to_string()}</p> }
                                .into_any()
                        }
                        Ok(page) => {
                            // Seed the cursor once the first page is known, so
                            // "more sons" continues from the right place.
                            if cursor.get_untracked().is_none() && !exhausted.get_untracked() {
                                match &page.next_cursor {
                                    Some(c) => set_cursor.set(Some(c.clone())),
                                    None => set_exhausted.set(true),
                                }
                            }

                            if page.sons.is_empty() {
                                return view! {
                                    <section class="empty">
                                        <h2>"No sons yet."</h2>
                                        <p>"The collection is empty. Be the first father."</p>
                                        <a class="btn" href="/upload">"contribute a son"</a>
                                    </section>
                                }
                                    .into_any();
                            }

                            view! {
                                <div class="grid">
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

        <div class="more">
            <Show
                when=move || !exhausted.get()
                fallback=|| view! { <p class="exhausted">"That's every son. For now."</p> }
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

/// The most-liked son from the last 24h (or overall, on a quiet day) —
/// computed on read in `db::son_of_the_day`, not curated by anyone.
#[component]
fn SonOfTheDay() -> impl IntoView {
    let featured = Resource::new(|| (), |_| son_of_the_day());

    view! {
        <Suspense fallback=|| ()>
            {move || {
                featured
                    .get()
                    .and_then(Result::ok)
                    .flatten()
                    .map(|s| {
                        let href = format!("/son/{}", s.id);
                        view! {
                            <a href=href class="sotd">
                                <img class="sotd-thumb" src=s.thumb_url.clone() alt=""/>
                                <div class="sotd-copy">
                                    <span class="sotd-label">"son of the day"</span>
                                    <span class="sotd-title">{s.title.clone()}</span>
                                </div>
                            </a>
                        }
                    })
            }}
        </Suspense>
    }
}
