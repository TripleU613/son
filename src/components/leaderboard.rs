use leptos::prelude::*;
use leptos_meta::{Link, Meta, Title};

use crate::api::leaderboard;
use crate::components::empty::{EmptyState, ErrorState};
use crate::components::icon::LuTrophy;
use crate::seo::absolute;

#[component]
pub fn Leaderboard() -> impl IntoView {
    let entries = Resource::new_blocking(|| (), |_| leaderboard());

    view! {
        <Title text="Leaderboard — son collection"/>
        <Meta
            name="description"
            content="Top contributors to the son collection."
        />
        <Link rel="canonical" href=absolute("/leaderboard")/>

        // No ranking explainer. The list itself communicates the ranking, and
        // the eligibility paragraph that used to sit here was pure narration.
        <h1 class="page-title">"Leaderboard"</h1>

        <Suspense fallback=|| view! { <p class="loading">"tallying…"</p> }>
            {move || {
                entries
                    .get()
                    .map(|res| match res {
                        Err(_) => {
                            view! { <ErrorState message="Something went wrong."/> }.into_any()
                        }
                        Ok(rows) if rows.is_empty() => {
                            view! { <EmptyState icon=LuTrophy message="No contributors yet."/> }
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
                                                {entry.upload_count}
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
