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
        <section class="hero">
            <h1>"report queue"</h1>
        </section>

        <Suspense fallback=|| view! { <p class="loading">"loading queue…"</p> }>
            {move || {
                flagged
                    .get()
                    .map(|res| match res {
                        Err(e) => {
                            view! {
                                <section class="empty">
                                    <h2>"Not available."</h2>
                                    <p>{e.to_string()}</p>
                                </section>
                            }
                                .into_any()
                        }
                        Ok(rows) if rows.is_empty() => {
                            view! {
                                <section class="empty">
                                    <h2>"Queue is empty."</h2>
                                    <p>"Nothing flagged right now."</p>
                                </section>
                            }
                                .into_any()
                        }
                        Ok(rows) => {
                            view! {
                                <div class="admin-queue">
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
        <article class="admin-row">
            <img class="admin-thumb" src=son.thumb_url.clone() alt=""/>
            <div class="admin-row-body">
                <div class="admin-row-head">
                    <a href=format!("/son/{}", son.id)>{son.title.clone()}</a>
                    <span class="admin-badge" class:public=son.is_public>
                        {if son.is_public { "visible" } else { "hidden" }}
                    </span>
                </div>
                <ul class="admin-reports">
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
                <div class="admin-actions">
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
                                    class="btn-quiet danger"
                                    on:click=move |_| set_confirming_delete.set(true)
                                >
                                    "delete"
                                </button>
                            }
                        }
                    >
                        <button
                            class="btn-quiet danger"
                            disabled=move || delete.pending().get()
                            on:click=move |_| {
                                delete.dispatch(());
                            }
                        >
                            "really delete? (no undo)"
                        </button>
                        <button class="link-btn" on:click=move |_| set_confirming_delete.set(false)>
                            "cancel"
                        </button>
                    </Show>
                </div>
            </div>
        </article>
    }
}
