use leptos::prelude::*;
use leptos_meta::Title;

use crate::api::{
    admin_delete_son, admin_flagged_sons, admin_screening_status, admin_set_gemini_cookies,
    admin_set_public,
};
use crate::components::empty::{EmptyState, ErrorState};
use crate::components::icon::{Ico, LuCheck, LuCircleAlert, LuRefreshCw};
use crate::models::FlaggedSon;

/// A destructive action, written out in full rather than as `btn-quiet` plus
/// overrides.
///
/// This replaced `.btn-quiet` plus two important-flagged hover colours, where
/// the flags were load-bearing: the primitive already sets a hover border and a
/// hover text colour, so without them the two rules sat at equal specificity and
/// the winner came down to their order in the generated stylesheet. That is the
/// precise failure mode this project deleted its stylesheet to escape, and
/// forcing it treats the symptom. Same geometry, its own hover colours, nothing
/// to fight.
///
/// The old class names are deliberately not written out anywhere above: the
/// Tailwind scanner reads these `.rs` files as plain text and does not skip
/// comments, so quoting them verbatim kept emitting the very rules this removed.
const BTN_DANGER: &str = "inline-flex min-h-9 items-center gap-2 rounded border border-line \
     bg-transparent px-3 text-[0.85rem] text-ink-2 transition-colors hover:border-danger \
     hover:bg-danger/10 hover:text-danger disabled:cursor-not-allowed disabled:opacity-50";

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
        <ScreeningPanel/>
        // Sentence case, like "Leaderboard" and "Search". This was the only
        // page title on the site in lowercase.
        <h1 class="m-0 mb-4 mt-8 text-[1.375rem] font-bold tracking-tight lg:mb-6 lg:text-[1.75rem]">"Report queue"</h1>

        <Suspense fallback=|| view! { <p class="py-14 text-center text-ink-2">"loading queue…"</p> }>
            {move || {
                flagged
                    .get()
                    .map(|res| match res {
                        // The shared states, not two more hand-rolled ones.
                        // Besides the inconsistency, the old error branch
                        // rendered `{e.to_string()}` straight into the page --
                        // the exact thing `ErrorState` was written to stop, and
                        // the raw server-fn wording is no more useful to an
                        // admin than to anyone else. The real error is still
                        // logged server-side.
                        Err(_) => {
                            view! {
                                <ErrorState
                                    message="Could not load the queue."
                                    retry_href="/admin"
                                />
                            }
                                .into_any()
                        }
                        Ok(rows) if rows.is_empty() => {
                            view! { <EmptyState icon=LuCheck message="Nothing flagged right now."/> }
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
                <div class="flex min-w-0 items-center gap-2.5">
                    <a
                        class="truncate font-medium transition-colors hover:text-accent"
                        href=format!("/son/{}", son.slug)
                    >
                        {son.title.clone()}
                    </a>
                    // Tinted rather than filled, and in theme colours. The
                    // filled version needed a hand-picked foreground for each
                    // state -- a raw `#06301c` on the green and white on the
                    // red -- which are the only two hardcoded hex values left
                    // anywhere in the UI and match nothing in the palette. A
                    // translucent fill lets `ok`/`danger` be both the text and
                    // the background, so the badge stays in the system.
                    <span class=if son.is_public {
                        "flex-none rounded-full border border-ok/30 bg-ok/15 px-2 py-0.5 text-[0.72rem] font-medium text-ok"
                    } else {
                        "flex-none rounded-full border border-danger/30 bg-danger/15 px-2 py-0.5 text-[0.72rem] font-medium text-danger"
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
                                    class=BTN_DANGER
                                    on:click=move |_| set_confirming_delete.set(true)
                                >
                                    "delete"
                                </button>
                            }
                        }
                    >
                        <button
                            class=BTN_DANGER
                            disabled=move || delete.pending().get()
                            on:click=move |_| {
                                delete.dispatch(());
                            }
                        >
                            "really delete? (no undo)"
                        </button>
                        // `btn-quiet`, like every other secondary control.
                        // Cancel was a bare unstyled <button> -- no height, no
                        // padding, no border -- sitting next to two real ones,
                        // so the safe way out of a destructive confirmation was
                        // the least clickable thing in the row.
                        <button
                            class="btn-quiet"
                            on:click=move |_| set_confirming_delete.set(false)
                        >
                            "cancel"
                        </button>
                    </Show>
                </div>

                // Failures used to be silent: both Actions dropped their Err
                // on the floor, so a hide or a delete that the server refused
                // looked exactly like one that worked -- the row simply stayed
                // as it was. On an admin surface that is the difference between
                // "already handled" and "still live".
                {move || {
                    let failed = toggle.value().get().is_some_and(|r| r.is_err())
                        || delete.value().get().is_some_and(|r| r.is_err());
                    failed
                        .then(|| {
                            view! {
                                <p class="m-0 text-[0.8rem] text-danger" role="alert">
                                    "That didn't go through. Try again."
                                </p>
                            }
                        })
                }}
            </div>
        </article>
    }
}

/// Screening status, and somewhere to paste fresh cookies.
///
/// This panel exists because the Gemini web session expires and there is no way
/// around that -- it authenticates as a browser session, and sessions die. What is
/// fixable is the cost: without this, restoring screening means editing a GitHub
/// secret and waiting out a ~12 minute CI deploy while uploads pile up in the held
/// queue. Here it is a paste and a click.
#[component]
fn ScreeningPanel() -> impl IntoView {
    let status = Resource::new(|| (), |_| admin_screening_status());
    let cookies: NodeRef<leptos::html::Textarea> = NodeRef::new();
    let submit = Action::new(|value: &String| {
        let value = value.clone();
        async move { admin_set_gemini_cookies(value).await }
    });

    // Re-read the status after a swap, so the panel shows the result of what was
    // just pasted rather than the state before it.
    Effect::new(move |_| {
        if submit.value().get().is_some() {
            status.refetch();
        }
    });

    view! {
        <section class="rounded-lg border border-line bg-surface p-4">
            <h2 class="m-0 mb-3 text-base font-semibold">"Screening"</h2>

            <Suspense fallback=|| view! { <p class="text-[0.85rem] text-ink-3">"checking…"</p> }>
                {move || {
                    status
                        .get()
                        .map(|res| match res {
                            Err(e) => {
                                view! { <p class="text-[0.85rem] text-danger">{e.to_string()}</p> }
                                    .into_any()
                            }
                            Ok(s) if !s.configured => {
                                view! {
                                    <p class="flex items-center gap-2 text-[0.9rem] text-ink-2">
                                        <span class="text-ink-3"><Ico icon=LuCircleAlert size=15/></span>
                                        "Off — GEMINI_URL is not set. Uploads publish unscreened."
                                    </p>
                                }
                                    .into_any()
                            }
                            Ok(s) if s.usable > 0 => {
                                view! {
                                    <p class="flex items-center gap-2 text-[0.9rem] text-ink">
                                        <span class="text-ok"><Ico icon=LuCheck size=15/></span>
                                        {format!("Working — {} account(s) answering.", s.usable)}
                                    </p>
                                }
                                    .into_any()
                            }
                            Ok(s) => {
                                // The important case: up, but nothing works. Says
                                // what happens to uploads meanwhile, because that
                                // is the actual consequence.
                                let detail = s
                                    .error
                                    .clone()
                                    .unwrap_or_else(|| {
                                        format!(
                                            "{} account(s) started, none answering — the cookies have expired.",
                                            s.initialised,
                                        )
                                    });
                                view! {
                                    <p class="flex items-center gap-2 text-[0.9rem] font-semibold text-danger">
                                        <span><Ico icon=LuCircleAlert size=15/></span>
                                        "Down"
                                    </p>
                                    <p class="m-0 mt-1 text-[0.85rem] text-ink-2">{detail}</p>
                                    <p class="m-0 mt-1 text-[0.85rem] text-ink-3">
                                        "Uploads are being held for review, not published."
                                    </p>
                                }
                                    .into_any()
                            }
                        })
                }}
            </Suspense>

            <form
                class="mt-3 grid gap-2"
                on:submit=move |ev| {
                    ev.prevent_default();
                    if let Some(t) = cookies.get() {
                        let v = t.value();
                        if !v.trim().is_empty() {
                            submit.dispatch(v);
                            t.set_value("");
                        }
                    }
                }
            >
                <label class="text-[0.8rem] text-ink-3" for="cookiebox">
                    "Fresh cookies as "
                    <code class="text-ink-2">"__Secure-1PSID:__Secure-1PSIDTS"</code>
                    ", comma-separated for several accounts"
                </label>
                <textarea
                    id="cookiebox"
                    node_ref=cookies
                    rows="3"
                    class="field font-mono text-[0.75rem]"
                    placeholder="g.a000...:sidts-..."
                />
                <div class="flex items-center gap-3">
                    <button class="btn" type="submit" disabled=move || submit.pending().get()>
                        <Ico icon=LuRefreshCw size=15/>
                        {move || if submit.pending().get() { "Applying…" } else { "Apply" }}
                    </button>
                    {move || {
                        submit
                            .value()
                            .get()
                            .map(|res| match res {
                                Ok(n) => {
                                    view! {
                                        <span class="text-[0.85rem] text-ok">
                                            {format!("{n} account(s) ready")}
                                        </span>
                                    }
                                        .into_any()
                                }
                                Err(e) => {
                                    view! {
                                        <span class="text-[0.85rem] text-danger">{e.to_string()}</span>
                                    }
                                        .into_any()
                                }
                            })
                    }}
                </div>
            </form>
        </section>
    }
}
