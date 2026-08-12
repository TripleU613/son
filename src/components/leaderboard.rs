use crate::app::SitePreview;
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
        <SitePreview/>
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
                            // `.enumerate()` over a plain iterator rather than
                            // <For>: the rank is the row's position, and <For>'s
                            // `let:entry` hands over the item without one. The
                            // list is a snapshot from a resolved Resource -- the
                            // whole branch re-renders when it changes -- so
                            // there is no keyed reconciliation to preserve here.
                            view! {
                                <ol class="m-0 grid list-none gap-2 p-0">
                                    {rows
                                        .into_iter()
                                        .enumerate()
                                        .map(|(i, entry)| {
                                            let rank = i + 1;
                                            // First place in the accent, the
                                            // rest quiet. Whole class strings,
                                            // not a toggled utility: the
                                            // Tailwind scanner reads these .rs
                                            // sources literally.
                                            let rank_class = if rank == 1 {
                                                "w-5 flex-none text-right text-[0.8125rem] font-semibold tabular-nums text-accent"
                                            } else {
                                                "w-5 flex-none text-right text-[0.8125rem] tabular-nums text-ink-3"
                                            };
                                            // Contributors who signed in without
                                            // a Google picture had no avatar at
                                            // all, so their name started 32px
                                            // left of everyone else's and the
                                            // column zig-zagged. Same initial
                                            // fallback the header account button
                                            // uses.
                                            let initial = entry
                                                .display_name
                                                .chars()
                                                .next()
                                                .map(|c| c.to_uppercase().to_string())
                                                .unwrap_or_else(|| "?".into());
                                            let unit = if entry.upload_count == 1 { "son" } else { "sons" };
                                            view! {
                                                <li class="flex items-center gap-3 rounded border border-line bg-surface px-3.5 py-2.5">
                                                    <span class=rank_class>{rank}</span>
                                                    {entry
                                                        .avatar_url
                                                        .clone()
                                                        .map(|src| {
                                                            view! {
                                                                <img class="h-8 w-8 flex-none rounded-full object-cover" src=src alt=""/>
                                                            }
                                                                .into_any()
                                                        })
                                                        .unwrap_or_else(|| {
                                                            view! {
                                                                <span class="flex h-8 w-8 flex-none items-center justify-center rounded-full bg-surface-raised text-[0.8rem] font-semibold text-ink-2">
                                                                    {initial.clone()}
                                                                </span>
                                                            }
                                                                .into_any()
                                                        })}
                                                    <span class="min-w-0 flex-1 truncate">
                                                        {entry.display_name.clone()}
                                                    </span>
                                                    // The bare number said
                                                    // nothing about what was
                                                    // being counted; the unit
                                                    // stays quiet so the figure
                                                    // still reads first.
                                                    <span class="flex-none text-[0.8125rem] text-ink-3">
                                                        <span class="font-semibold tabular-nums text-accent">
                                                            {entry.upload_count}
                                                        </span>
                                                        {format!(" {unit}")}
                                                    </span>
                                                </li>
                                            }
                                        })
                                        .collect_view()}
                                </ol>
                            }
                                .into_any()
                        }
                    })
            }}
        </Suspense>
    }
}
