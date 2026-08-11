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
        <h1 class="m-0 mb-4 text-[1.375rem] font-bold tracking-tight lg:mb-6 lg:text-[1.75rem]">"Leaderboard"</h1>

        <Suspense fallback=|| view! { <p class="py-14 text-center text-ink-2">"tallying…"</p> }>
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
                                <ol class="m-0 grid list-none gap-2 p-0">
                                    <For each=move || rows.clone() key=|r| r.display_name.clone() let:entry>
                                        <li class="flex items-center gap-3 rounded border border-line bg-surface px-3.5 py-2.5">
                                            {entry
                                                .avatar_url
                                                .clone()
                                                .map(|src| {
                                                    view! {
                                                        <img class="h-8 w-8 flex-none rounded-full object-cover" src=src alt=""/>
                                                    }
                                                })}
                                            <span class="flex-1">{entry.display_name.clone()}</span>
                                            <span class="text-[0.9rem] tabular-nums text-accent">
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
