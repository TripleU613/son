use leptos::prelude::*;
use leptos_meta::Title;

use crate::api::{admin_delete_son, admin_flagged_sons, admin_set_public};
use crate::models::FlaggedSon;

/// The report queue. Gated server-side in every server fn it calls
/// (`api::require_admin`) — this component hiding itself from a non-admin is
/// a courtesy, not the actual access control.
#[component]
pub fn Admin() -> impl IntoView {
    let flagged = Resource::new_blocking(|| (), |_| admin_flagged_sons());
    // Bumped after any action to force a refetch, since the three admin
    // actions below are plain Actions, not tied to this Resource directly.
    let (refresh, set_refresh) = signal(0u32);
    Effect::new(move |_| {
        refresh.get();
        flagged.refetch();
    });

    view! {
        <Title text="admin — son collection"/>
        <h1 class="m-0 mb-4 text-[1.375rem] font-bold tracking-tight lg:mb-6 lg:text-[1.75rem]">"report queue"</h1>

        <Suspense fallback=|| view! { <p class="py-14 text-center text-ink-2">"loading queue…"</p> }>
            {move || {
                flagged
                    .get()
                    .map(|res| match res {
                        Err(e) => {
                            view! {
                                <section class="flex min-h-[46vh] flex-col items-center justify-center gap-3 text-center">
                                    <h2>"Not available."</h2>
                                    <p>{e.to_string()}</p>
                                </section>
                            }
                                .into_any()
                        }
                        Ok(rows) if rows.is_empty() => {
                            view! {
                                <section class="flex min-h-[46vh] flex-col items-center justify-center gap-3 text-center">
                                    <h2>"Queue is empty."</h2>
                                    <p>"Nothing flagged right now."</p>
                                </section>
                            }
                                .into_any()
                        }
                        Ok(rows) => {
                            view! {
                                <div class="grid gap-3.5">
                                    <For each=move || rows.clone() key=|f| f.son.id.clone() let:flagged>
                                        <AdminRow flagged=flagged on_change=move || set_refresh.update(|n| *n += 1)/>
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

#[component]
fn AdminRow(
    flagged: FlaggedSon,
    on_change: impl Fn() + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let son = flagged.son;
    let (confirming_delete, set_confirming_delete) = signal(false);
    let is_public = son.is_public;
    let id_for_toggle = son.id.clone();
    let id_for_delete = son.id.clone();

    let toggle = Action::new(move |()| {
        let id = id_for_toggle.clone();
        async move { admin_set_public(id, !is_public).await }
    });
    let delete = Action::new(move |()| {
        let id = id_for_delete.clone();
        async move { admin_delete_son(id).await }
    });

    // Both actions refetch the queue on success, from a plain Effect rather
    // than chaining inside the Action itself — simpler than threading the
    // parent's refetch through two separate async closures.
    Effect::new(move |_| {
        if toggle.value().get().is_some_and(|r| r.is_ok()) {
            on_change();
        }
    });
    Effect::new(move |_| {
        if delete.value().get().is_some_and(|r| r.is_ok()) {
            on_change();
        }
    });

    view! {
        <article class="flex flex-col gap-3.5 rounded-lg border border-line bg-surface p-3.5 sm:flex-row">
            <img class="h-[88px] w-[88px] flex-none rounded object-cover" src=son.thumb_url.clone() alt=""/>
            <div class="grid min-w-0 flex-1 gap-2">
                <div class="flex items-center gap-2.5">
                    <a href=format!("/son/{}", son.slug)>{son.title.clone()}</a>
                    <span class=if son.is_public {
                        "rounded-full bg-ok px-2 py-0.5 text-[0.72rem] text-[#06301c]"
                    } else {
                        "rounded-full bg-danger px-2 py-0.5 text-[0.72rem] text-white"
                    }>
                        {if son.is_public { "visible" } else { "hidden" }}
                    </span>
                </div>
                <ul class="m-0 pl-5 text-[0.85rem] text-ink-2">
                    <For
                        each={
                            let reports = flagged.reports.clone();
                            move || reports.clone()
                        }
                        key=|r| format!("{}{}", r.reason, r.created_at)
                        let:r
                    >
                        <li>
                            <strong>{r.reason.clone()}</strong>
                            {r.message.clone().map(|m| view! { <span>": " {m}</span> })}
                        </li>
                    </For>
                </ul>
                <div class="flex items-center gap-2.5">
                    <button
                        class="btn-quiet"
                        disabled=move || toggle.pending().get()
                        on:click=move |_| {
                            toggle.dispatch(());
                        }
                    >
                        {if is_public { "hide" } else { "unhide" }}
                    </button>
                    <Show
                        when=move || confirming_delete.get()
                        fallback=move || {
                            view! {
                                <button
                                    class="btn-quiet hover:!border-danger hover:!text-danger"
                                    on:click=move |_| set_confirming_delete.set(true)
                                >
                                    "delete"
                                </button>
                            }
                        }
                    >
                        <button
                            class="btn-quiet hover:!border-danger hover:!text-danger"
                            disabled=move || delete.pending().get()
                            on:click=move |_| {
                                delete.dispatch(());
                            }
                        >
                            "really delete? (no undo)"
                        </button>
                        <button class="cursor-pointer border-0 bg-transparent p-0 text-inherit hover:text-danger" on:click=move |_| set_confirming_delete.set(false)>
                            "cancel"
                        </button>
                    </Show>
                </div>
            </div>
        </article>
    }
}
