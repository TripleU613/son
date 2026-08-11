use leptos::prelude::*;
use leptos_meta::{Link, Meta, Title};

use crate::api::leaderboard;
use crate::seo::absolute;

#[component]
pub fn Leaderboard() -> impl IntoView {
    let entries = Resource::new_blocking(|| (), |_| leaderboard());

    view! {
        <Title text="leaderboard — son collection"/>
        <Meta
            name="description"
            content="Top contributors to the son collection, ranked by public sons uploaded."
        />
        <Link rel="canonical" href=absolute("/leaderboard")/>

        <section class="hero">
            <h1>"contributors"</h1>
            <p>"Ranked by public sons uploaded while signed in. Anonymous uploads don't count toward this — sign in for the credit."</p>
        </section>

        <Suspense fallback=|| view! { <p class="loading">"tallying…"</p> }>
            {move || {
                entries
                    .get()
                    .map(|res| match res {
                        Err(e) => view! { <p class="error">{e.to_string()}</p> }.into_any(),
                        Ok(rows) if rows.is_empty() => {
                            view! {
                                <section class="empty">
                                    <h2>"No contributors yet."</h2>
                                    <p>"Sign in and upload a son to be the first."</p>
                                </section>
                            }
                                .into_any()
                        }
                        Ok(rows) => {
                            view! {
                                <ol class="leaderboard">
                                    <For each=move || rows.clone() key=|r| r.display_name.clone() let:entry>
                                        <li class="leaderboard-row">
                                            {entry
                                                .avatar_url
                                                .clone()
                                                .map(|src| {
                                                    view! {
                                                        <img class="leaderboard-avatar" src=src alt=""/>
                                                    }
                                                })}
                                            <span class="leaderboard-name">{entry.display_name.clone()}</span>
                                            <span class="leaderboard-count">
                                                {entry.upload_count} " sons"
                                            </span>
                                        </li>
                                    </For>
                                </ol>
                            }
                                .into_any()
                        }
                    })
            }}
        </Suspense>
    }
}
