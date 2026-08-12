use crate::app::SitePreview;
use leptos::prelude::*;
use leptos_meta::{Link, Meta, Title};

use crate::api::leaderboard;
use crate::components::density::{stagger, RowSkeleton};
use crate::components::empty::{EmptyState, ErrorState};
use crate::components::icon::LuTrophy;
use crate::seo::absolute;

/// The rank number's colour, as three whole class strings.
///
/// A podium rather than a single winner: first place in the full accent, second
/// and third in the muted one, everyone else quiet. The middle arm is the point
/// of the change -- with two arms, ranks 2 and 3 were styled identically to
/// rank 40, so the top of the list had no shape to it.
///
/// Whole strings, not a toggled utility, because the Tailwind scanner reads
/// these `.rs` sources literally.
fn rank_class(rank: usize) -> &'static str {
    match rank {
        1 => "w-5 flex-none text-right text-[0.8125rem] font-semibold tabular-nums text-accent",
        2 | 3 => {
            "w-5 flex-none text-right text-[0.8125rem] font-semibold tabular-nums text-accent-muted"
        }
        _ => "w-5 flex-none text-right text-[0.8125rem] tabular-nums text-ink-3",
    }
}

/// The row's own surface. First place gets a faint accent wash and hairline so
/// the top of the list is a warm band instead of one more grey row; the colour
/// is never the only signal, since the rank number and its weight already say
/// the same thing.
fn row_class(rank: usize) -> &'static str {
    if rank == 1 {
        "flex items-center gap-3 rounded border border-accent-line bg-accent-veil px-3.5 py-2.5"
    } else {
        "flex items-center gap-3 rounded border border-line bg-surface px-3.5 py-2.5"
    }
}

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

        // A placeholder shaped like the list, rather than a word where the list
        // will be: "tallying…" is one line tall, so the page grew by six rows
        // the moment the tally arrived.
        <Suspense fallback=|| view! { <RowSkeleton count=6/> }>
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
                                            // Every interpolated piece is
                                            // itself a whole literal, so the
                                            // scanner still sees each one in
                                            // full somewhere in the source.
                                            let li_class = format!(
                                                "{} {} {}",
                                                row_class(rank),
                                                "animate-rise-in",
                                                stagger(i),
                                            );
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
                                                <li class=li_class>
                                                    <span class=rank_class(rank)>{rank}</span>
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
