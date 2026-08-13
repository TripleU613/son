use leptos::prelude::*;
use leptos_meta::{Link, Meta, Title};
use leptos_router::components::A;
use leptos_router::hooks::{use_navigate, use_params_map};

use crate::api::{admin_delete_son, current_user, get_son, son_neighbours};
use crate::components::icon::{Ico, LuCircleAlert, LuDownload, LuTrash2, LuUserRound};
use crate::components::like::LikeButton;
use crate::components::more_sons::MoreSons;
use crate::components::report::ReportForm;
use crate::components::share::ShareButton;
use crate::models::Son;
use crate::seo::{absolute, json_escape};

/// A one-line, human-readable summary of a son -- reused as the page's
/// `<meta name="description">`, `og:description`, and `twitter:description`
/// so the three don't drift out of sync with each other.
fn describe(s: &Son) -> String {
    let by = match &s.uploader {
        Some(u) => format!(" Contributed by {}.", u.display_name),
        None => String::new(),
    };
    format!("{} — in the son collection.{}", s.title, by)
}

/// `2026-08-12` -> `Aug 12, 2026`, for the byline under a son's title.
///
/// Hand-parsed rather than through `chrono`, which is an ssr-only dependency in
/// Cargo.toml while this component also compiles for wasm. A stored timestamp is
/// already UTC and only its date half is shown, so there is no timezone maths
/// here worth a dependency that half the builds cannot see anyway.
///
/// Anything not shaped like `YYYY-MM-DD` falls through to the raw string: a
/// visitor seeing an ISO date is a cosmetic disappointment, a visitor seeing an
/// invented one is a bug.
fn pretty_date(ymd: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    let mut parts = ymd.split('-');
    let (Some(y), Some(m), Some(d), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return ymd.to_string();
    };
    let (Ok(month), Ok(day)) = (m.parse::<usize>(), d.parse::<u32>()) else {
        return ymd.to_string();
    };
    // wrapping_sub so month 0 underflows to a huge index and misses, rather
    // than panicking on 0 - 1.
    match MONTHS.get(month.wrapping_sub(1)) {
        Some(name) if y.len() == 4 => format!("{name} {day}, {y}"),
        _ => ymd.to_string(),
    }
}

/// `schema.org/ImageObject` JSON-LD. Structured data is one of the few
/// levers Google documents explicitly for ranking in Google Images, so this
/// exists specifically to help this page dominate there, not just Discord/
/// Twitter previews (which only need the OG tags above).
///
/// Hand-built rather than via `serde_json` (unavailable in the wasm/hydrate
/// build this component also compiles under) -- every string field goes
/// through `json_escape`, which also neutralizes `</script>` so a title or
/// uploader name can never break out of the tag it's embedded in.
fn image_object_json_ld(s: &Son) -> String {
    let creator = s
        .uploader
        .as_ref()
        .map(|u| {
            format!(
                r#","creator":{{"@type":"Person","name":"{}"}}"#,
                json_escape(&u.display_name)
            )
        })
        .unwrap_or_default();
    format!(
        r#"{{"@context":"https://schema.org","@type":"ImageObject","contentUrl":"{content_url}","thumbnailUrl":"{thumb_url}","name":"{name}","description":"{description}","uploadDate":"{uploaded}","width":{width},"height":{height}{creator}}}"#,
        content_url = json_escape(&s.orig_url),
        thumb_url = json_escape(&s.thumb_url),
        name = json_escape(&s.title),
        description = json_escape(&describe(s)),
        uploaded = json_escape(&s.created_at),
        width = s.width,
        height = s.height,
    )
}

/// How far a finger has to travel sideways before letting go steps to the
/// neighbouring son.
///
/// A fixed distance rather than a fraction of the figure's width, because that
/// width is only knowable from the DOM and the server render has no
/// `getBoundingClientRect` to agree with. Anything measured would have to be
/// seeded on the client after mount, which is the hydration mismatch this file
/// cannot afford.
const SWIPE_COMMIT_PX: f64 = 64.0;

/// Past this much vertical travel the gesture is the page scrolling, not a
/// swipe, and it is handed back rather than competed with.
const SWIPE_ABANDON_Y: f64 = 24.0;

/// How far the son may be dragged when there is no son that way.
///
/// Resistance rather than a wall: it still moves, so the gesture is visibly
/// understood *and* visibly refused, which is what tells someone they have
/// reached the end of the collection. A drag that did nothing at all would read
/// as the feature being broken.
const SWIPE_RUBBER_PX: f64 = 28.0;

// None of the three is `#[cfg]`-gated on purpose. They are read by pointer
// handlers that compile under both feature sets — the handlers are inert on the
// server, but they are still built there, so gating these would break the ssr
// build rather than quiet it.

/// Delete this son. Only rendered for an admin, and only in the browser.
///
/// The signed-in user is resolved in an `Effect` rather than a `Resource`, for
/// the same reason spelled out on the neighbour lookup below: this route is
/// `SsrMode::Async`, which holds the whole response until every resource on the
/// page has resolved, and the only reason it is Async at all is to get the og:
/// tags into the server HTML. A resource here would put a D1 round trip in front
/// of time-to-first-byte on the one page whose entire job is being unfurled in a
/// chat client -- to decide whether to draw a button that almost nobody can see.
/// Effects are client-only by construction, so that cost lands only where the
/// control can actually appear.
///
/// The access control is `admin_delete_son`'s own `require_admin`, server-side.
/// This component hiding itself from everyone else is a courtesy, exactly as the
/// /admin page documents about itself.
#[component]
fn AdminDelete(id: String, slug: String) -> impl IntoView {
    let (is_admin, set_is_admin) = signal(false);
    let (confirming, set_confirming) = signal(false);
    let (failed, set_failed) = signal(false);
    // StoredValue, not the String: this is read from inside click handlers that
    // live in a reactive view, so a captured String would make them FnOnce.
    let son_id = StoredValue::new(id);
    let son_slug = StoredValue::new(slug);
    let navigate = use_navigate();

    Effect::new(move |_| {
        leptos::task::spawn_local(async move {
            // An error here means "could not reach D1", which is not the same as
            // "not an admin" -- but it resolves to the same thing on screen, and
            // the safe direction is to show nothing.
            if let Ok(Some(user)) = current_user().await {
                set_is_admin.set(user.is_admin);
            }
        });
    });

    view! {
        <Show when=move || is_admin.get() fallback=|| ()>
            {
                let navigate = navigate.clone();
                view! {
                    <Show
                        when=move || confirming.get()
                        fallback=move || {
                            view! {
                                // `.icon-btn`'s geometry written out in full
                                // rather than the class plus hover overrides.
                                // The primitive already sets a hover background
                                // and a hover text colour, so overriding those
                                // is two rules for one property decided by
                                // stylesheet order -- the cascade coin-flip this
                                // project deleted its stylesheet to escape, and
                                // the thing the admin queue needed !important
                                // for until that was rewritten the same way.
                                <button
                                    type="button"
                                    class="inline-flex h-9 w-9 items-center justify-center rounded border border-transparent bg-transparent text-ink-2 transition-colors hover:border-danger hover:bg-danger/10 hover:text-danger"
                                    aria-label="Delete this son"
                                    title="Delete this son"
                                    on:click=move |_| set_confirming.set(true)
                                >
                                    <Ico icon=LuTrash2 size=17/>
                                </button>
                            }
                        }
                    >
                        {
                            let navigate = navigate.clone();
                            view! {
                                // Two steps, like the admin queue's own delete:
                                // this removes the R2 objects as well as the row,
                                // so there is nothing to undo afterwards.
                                <button
                                    type="button"
                                    class="inline-flex min-h-9 items-center gap-2 rounded border border-danger bg-danger/10 px-3 text-[0.85rem] font-semibold text-danger transition-colors hover:bg-danger/20"
                                    on:click=move |_| {
                                        let navigate = navigate.clone();
                                        set_failed.set(false);
                                        leptos::task::spawn_local(async move {
                                            match admin_delete_son(son_id.get_value()).await {
                                                // Gone, so staying on its page
                                                // would render a 404 for the
                                                // thing just deleted.
                                                Ok(()) => navigate("/", Default::default()),
                                                Err(e) => {
                                                    leptos::logging::error!(
                                                        "delete failed for {}: {e}", son_slug.get_value()
                                                    );
                                                    set_failed.set(true);
                                                }
                                            }
                                        });
                                    }
                                >
                                    <Ico icon=LuTrash2 size=15/>
                                    "really delete? (no undo)"
                                </button>
                                <button
                                    type="button"
                                    class="btn-quiet"
                                    on:click=move |_| set_confirming.set(false)
                                >
                                    "cancel"
                                </button>
                            }
                        }
                    </Show>
                }
            }
        </Show>
        <Show when=move || failed.get() fallback=|| ()>
            <p class="m-0 basis-full text-[0.8rem] text-danger" role="alert">
                "That didn't go through. Try again."
            </p>
        </Show>
    }
}

/// A single son's page.
///
/// This is the URL people paste into Discord and group chats, so the OG tags
/// matter as much as the layout — they render server-side, before hydration.
#[component]
pub fn SonDetail() -> impl IntoView {
    let params = use_params_map();
    let id = move || params.read().get("id").unwrap_or_default();

    // Blocking, not a plain Resource: the og:/twitter: tags below depend on this
    // data, and with out-of-order streaming the <head> flushes before the
    // resource resolves — link unfurlers would get a bare title and no image.
    // Blocking holds the stream until the son is known.
    let son = Resource::new_blocking(id, |id| async move { get_son(id).await });

    // Swipe state, declared here rather than inside the `son.get().map(...)`
    // closure below. That closure re-runs on every change to the resource, so
    // signals minted inside it would be brand new after each step and the ones
    // the pointer handlers had already captured would be orphaned — the gesture
    // would work exactly once.
    //
    // `drag_x` is how far the son currently follows the finger, `settling` is
    // whether it is animating back to rest, `grab` is where the finger landed.
    // All three start at values the server can print, which is what keeps
    // hydration intact: seeding any of them from `window`, a capability probe or
    // a resource would kill the wasm module and leave the page inert rather than
    // merely mis-drawn.
    let (drag_x, set_drag_x) = signal(0.0_f64);
    let (settling, set_settling) = signal(false);
    let (grab, set_grab) = signal(Option::<(f64, f64)>::None);
    let (prev_slug, set_prev_slug) = signal(Option::<String>::None);
    let (next_slug, set_next_slug) = signal(Option::<String>::None);
    // Safe here: `SonDetail` only ever mounts inside `<Router>`, and this is a
    // documented no-op during SSR. It does mean the component can no longer be
    // rendered standalone in a test without a Router around it.
    let navigate = use_navigate();

    // The neighbouring slugs, in an `Effect` and deliberately not a `Resource`.
    //
    // This route is `SsrMode::Async`, which holds the whole response until every
    // resource on it resolves — and the only reason it is Async is that the og:
    // tags have to be in the server HTML. A second resource here would put a D1
    // round trip in front of the time-to-first-byte of the one page whose entire
    // job is being unfurled in a chat client, to feed a touch gesture that cannot
    // exist during SSR at all. Effects are client-only by construction, so the
    // cost lands only where the feature is usable.
    Effect::new(move |_| {
        let id = id();
        // Cleared synchronously, before the request goes out: mid-fetch these
        // still hold the *previous* son's neighbours, and a swipe that commits
        // against them navigates somewhere with no relation to what is on
        // screen. The drag is reset for the same reason — the gesture that
        // brought us here is over.
        set_prev_slug.set(None);
        set_next_slug.set(None);
        set_drag_x.set(0.0);
        set_settling.set(false);
        leptos::task::spawn_local(async move {
            match son_neighbours(id).await {
                Ok((newer, older)) => {
                    set_prev_slug.set(newer);
                    set_next_slug.set(older);
                }
                // Warn, don't surface. The grid below is still a way to another
                // son, so the only visible consequence is that the gesture
                // rubber-bands in both directions instead of stepping.
                Err(e) => leptos::logging::warn!("neighbours unavailable: {e}"),
            }
        });
    });

    view! {
        // `Transition`, not `Suspense`: every committed swipe re-runs the
        // resource, and under `Suspense` that blanks the whole article back to
        // "finding the son…" for a D1 round trip — the opposite of a continuous
        // gesture. `Transition` holds the son already on screen until the next
        // one lands. Server-side the two are the same code path (both are
        // `SuspenseBoundary`; only the client rebuild branches on it), so the og:
        // tags stay in the HTML rather than moving into a JS-swapped <template>.
        <Transition fallback=|| view! { <p class="py-14 text-center text-ink-2">"finding the son…"</p> }>
            {move || {
                son.get()
                    .map(|res| match res {
                        Err(e) => {
                            view! { <p class="text-danger">{e.to_string()}</p> }.into_any()
                        }
                        Ok(None) => {
                            // A real 404, not the body alone: this URL is the
                            // shape people paste around, so a typo'd or deleted
                            // son must not be indexable as a page that exists.
                            crate::app::set_status(404);
                            view! {
                                <section class="flex min-h-[46vh] flex-col items-center justify-center gap-3 text-center">
                                    <h1>"No such son."</h1>
                                    <A href="/">"back to the collection"</A>
                                </section>
                            }
                                .into_any()
                        }
                        Ok(Some(s)) => {
                            let description = describe(&s);
                            let page_url = absolute(&format!("/son/{}", s.slug));
                            let json_ld = image_object_json_ld(&s);
                            // The date half of the stored timestamp, kept as-is
                            // for the <time datetime> attribute and formatted
                            // separately for display.
                            let iso_date: String = s.created_at.chars().take(10).collect();
                            // Bound out here rather than written inside the
                            // view: the macro captures by move, so a `.clone()`
                            // spelled inside it is already too late for the use
                            // that follows (the lesson card.rs learned).
                            let uploader_name = match &s.uploader {
                                Some(u) => u.display_name.clone(),
                                None => "anonymous".to_string(),
                            };
                            let uploader_avatar = s
                                .uploader
                                .as_ref()
                                .and_then(|u| u.avatar_url.clone());

                            // The swipe, wired to the <figure> below.
                            //
                            // Pointer events, not touch events:
                            // `TouchEvent::touches()` is gated on web-sys's
                            // `TouchList` feature, which nothing in this tree
                            // turns on, so touch would mean a Cargo.toml edit.
                            // `PointerEvent` costs nothing because tachys
                            // already enables it for both builds — the same
                            // arrangement like.rs relies on for `MouseEvent` and
                            // upload.rs for `DragEvent`. If a future leptos bump
                            // trims that feature list this breaks with "no
                            // method named pointer_type" rather than with a
                            // missing-feature error, so look here first.
                            //
                            // There is no `setPointerCapture` and no
                            // `preventDefault`. The spec gives touch pointers
                            // implicit capture on pointerdown, so move and up
                            // keep arriving at the figure after the finger
                            // leaves it, and the figure's own touch-action
                            // already stops the browser panning sideways, so
                            // there is no default left to cancel. Where implicit
                            // capture is missing the moves simply stop and the
                            // swipe never commits — degrading to exactly today's
                            // behaviour, which is the required way for this to
                            // fail.
                            //
                            // These are built here, not at the top of the
                            // component: the closure around this match has to
                            // stay `Fn` to re-render, and moving a handler into
                            // it would make it `FnOnce`. Cloning `navigate` per
                            // render only borrows the outer one, so the closure
                            // keeps its `Fn`.
                            let navigate = navigate.clone();

                            let on_down = move |ev: leptos::ev::PointerEvent| {
                                // A capability test, not a size test.
                                // `window.innerWidth` would disagree between the
                                // server render and the client and take the wasm
                                // module down with it; `pointer_type` is only
                                // ever read inside a handler, which the server
                                // never runs. A mouse falls straight through and
                                // keeps click-drag and save-image.
                                if ev.pointer_type() != "touch" {
                                    return;
                                }
                                set_settling.set(false);
                                set_grab
                                    .set(
                                        Some((
                                            f64::from(ev.client_x()),
                                            f64::from(ev.client_y()),
                                        )),
                                    );
                            };

                            let on_move = move |ev: leptos::ev::PointerEvent| {
                                let Some((x0, y0)) = grab.get_untracked() else {
                                    return;
                                };
                                let dx = f64::from(ev.client_x()) - x0;
                                let dy = f64::from(ev.client_y()) - y0;
                                // Mostly-vertical travel is the page scrolling.
                                if dy.abs() > SWIPE_ABANDON_Y && dy.abs() > dx.abs() {
                                    set_grab.set(None);
                                    set_settling.set(true);
                                    set_drag_x.set(0.0);
                                    return;
                                }
                                let has_target = if dx < 0.0 {
                                    next_slug.get_untracked().is_some()
                                } else {
                                    prev_slug.get_untracked().is_some()
                                };
                                // Scaled first and clamped second, so the
                                // resistance is felt from the first pixel rather
                                // than after 28 free ones.
                                set_drag_x
                                    .set(
                                        if has_target {
                                            dx
                                        } else {
                                            (dx * 0.4)
                                                .clamp(-SWIPE_RUBBER_PX, SWIPE_RUBBER_PX)
                                        },
                                    );
                            };

                            let on_up = move |_: leptos::ev::PointerEvent| {
                                if grab.get_untracked().is_none() {
                                    return;
                                }
                                set_grab.set(None);
                                let dx = drag_x.get_untracked();
                                set_settling.set(true);
                                set_drag_x.set(0.0);
                                if dx.abs() < SWIPE_COMMIT_PX {
                                    return;
                                }
                                // Left goes to the older son, right to the newer
                                // one: the direction every photo viewer uses,
                                // and the same order the grid below is in.
                                let target = if dx < 0.0 {
                                    next_slug.get_untracked()
                                } else {
                                    prev_slug.get_untracked()
                                };
                                if let Some(slug) = target {
                                    navigate(&format!("/son/{slug}"), Default::default());
                                }
                            };

                            let on_cancel = move |_: leptos::ev::PointerEvent| {
                                set_grab.set(None);
                                set_settling.set(true);
                                set_drag_x.set(0.0);
                            };

                            // Two whole class strings, never a transition
                            // utility layered onto the resting one (the like.rs
                            // pattern): layered, both land at equal specificity
                            // and stylesheet order decides. While the finger is
                            // down there must be no transition at all or the son
                            // lags behind it; the settle back to rest is the
                            // only animated state, and prefers-reduced-motion
                            // needs nothing new here because style/tailwind.css
                            // already zeroes every transition globally.
                            let img_class = move || {
                                if settling.get() {
                                    "h-auto max-h-[calc(68vh-1.5rem)] w-auto max-w-full rounded object-contain transition-transform duration-200 ease-out"
                                } else {
                                    "h-auto max-h-[calc(68vh-1.5rem)] w-auto max-w-full rounded object-contain"
                                }
                            };
                            view! {
                                <Title text=format!("{} — son collection", s.title)/>
                                <Meta name="description" content=description.clone()/>
                                <Link rel="canonical" href=page_url.clone()/>

                                <Meta property="og:title" content=s.title.clone()/>
                                <Meta property="og:description" content=description.clone()/>
                                <Meta property="og:image" content=absolute(&s.orig_url)/>
                                <Meta property="og:image:width" content=s.width.to_string()/>
                                <Meta property="og:image:height" content=s.height.to_string()/>
                                <Meta property="og:type" content="image"/>
                                <Meta property="og:url" content=page_url.clone()/>

                                <Meta name="twitter:card" content="summary_large_image"/>
                                <Meta name="twitter:title" content=s.title.clone()/>
                                <Meta name="twitter:description" content=description/>
                                <Meta name="twitter:image" content=absolute(&s.orig_url)/>

                                // schema.org/ImageObject structured data -- see
                                // `image_object_json_ld`'s doc comment for why
                                // this exists.
                                <script type="application/ld+json" inner_html=json_ld/>

                                // oEmbed discovery: lets embedders that check
                                // <link rel="alternate"> (WordPress, many wikis)
                                // find this without knowing the endpoint shape
                                // up front, the way Discord/Twitter's OG parsing
                                // already does implicitly.
                                <Link
                                    rel="alternate"
                                    type_="application/json+oembed"
                                    title=s.title.clone()
                                    href=format!("{}?url={}", absolute("/oembed"), page_url)
                                />

                                // Flex row that centres the pair, not a
                                // 1fr-plus-sidebar grid.
                                //
                                // The grid was the distortion. Its first track
                                // took every pixel the 300px sidebar did not,
                                // and the figure -- correctly capped so it never
                                // upscales -- then centred itself inside that
                                // track. At 1440 that left a 612px image adrift
                                // in a 994px column with ~190px of dead
                                // background on each side and the text panel
                                // stranded against the far right edge, so the
                                // two halves of the page did not look related.
                                //
                                // As a centred flex row the figure is only as
                                // wide as the son, the panel is a fixed column
                                // beside it, and the pair sits together in the
                                // middle whatever the son's size. The figure
                                // keeps the default `flex: 0 1 auto` so it can
                                // still shrink between 860px and ~1100px, where
                                // image plus panel would otherwise overflow.
                                <article class="flex flex-col gap-4 pb-4 min-[860px]:flex-row min-[860px]:items-start min-[860px]:justify-center min-[860px]:gap-8 min-[860px]:pb-6">
                                    // Constrained figure, not a full-bleed
                                    // image: capped at 68vh so the son is
                                    // shown whole without pushing everything
                                    // else off-screen, and `contain` so a tall
                                    // son is letterboxed rather than cropped.
                                    //
                                    // All three caps are needed: the container
                                    // (100%), the son's own pixel width (never
                                    // upscale), and the width the 68vh height
                                    // cap implies for this aspect (which is what
                                    // a portrait son is actually limited by).
                                    // `min()` takes the tightest.
                                    //
                                    // `mx-auto` centres it on mobile, and is
                                    // cancelled at 860px: auto margins on a flex
                                    // item swallow all the free space, which
                                    // would override the row's own centring and
                                    // shove the panel back out to the edge --
                                    // reintroducing the exact gap this change
                                    // removes.
                                    //
                                    // The gesture handlers live on the figure
                                    // and nowhere else. No control sits inside
                                    // it, so a drag can never swallow a
                                    // button's click. `pan-y pinch-zoom` rather
                                    // than plain `pan-y` because pinching a meme
                                    // is the one thing a visitor may
                                    // legitimately want to do to it; the
                                    // horizontal pan the browser would otherwise
                                    // take for scrolling is the only thing given
                                    // up, and it is what the swipe is made of.
                                    <figure
                                        class="m-0 mx-auto flex max-h-[68vh] select-none items-center justify-center overflow-hidden rounded-lg border border-line bg-surface p-3 [touch-action:pan-y_pinch-zoom] min-[860px]:mx-0"
                                        style=format!(
                                            "max-width: min(100%, calc({}px + 1.5rem), calc(68vh * {:.4} + 1.5rem))",
                                            s.width,
                                            f64::from(s.width.max(1)) / f64::from(s.height.max(1)),
                                        )
                                        on:pointerdown=on_down
                                        on:pointermove=on_move
                                        on:pointerup=on_up
                                        on:pointercancel=on_cancel
                                    >
                                        // The son carries the motion, not the
                                        // frame: it slides inside the figure's
                                        // own overflow-hidden box, which reads
                                        // as looking through a window rather
                                        // than shoving one. It also keeps the
                                        // figure's static `style=` attribute
                                        // clear of a reactive `style:transform`.
                                        //
                                        // `{:.1}` so the server prints exactly
                                        // `translateX(0.0px)` and the hydrating
                                        // client computes the identical string.
                                        <img
                                            class=img_class
                                            style:transform=move || {
                                                format!("translateX({:.1}px)", drag_x.get())
                                            }
                                            src=s.orig_url.clone()
                                            alt=s.title.clone()
                                            width=s.width
                                            height=s.height
                                        />
                                    </figure>

                                    // Fixed column beside the son on desktop,
                                    // full width beneath it on mobile.
                                    // `flex-none` so it never shrinks: the
                                    // figure is the thing with room to give.
                                    <div class="min-w-0 min-[860px]:w-[320px] min-[860px]:flex-none">
                                        // Title, credit and controls are one
                                        // block, not three things at arm's
                                        // length. Measured before: 12px from
                                        // title to credit but 36px from credit
                                        // to the first control, so the credit
                                        // floated between them belonging to
                                        // neither. It belongs to the title, and
                                        // the spacing now says so -- 4px up to
                                        // the title, 16px down to the bar.
                                        //
                                        // `leading-tight` because at the 1.5
                                        // inherited from `body` a two-line title
                                        // opens a trough right through the son's
                                        // own name. That is a utility against an
                                        // inherited value, not a
                                        // same-specificity fight with a
                                        // primitive.
                                        <h1 class="m-0 text-[1.375rem] font-bold leading-tight tracking-tight lg:text-[1.75rem]">{s.title.clone()}</h1>

                                        // Only an admin can reach a son that is
                                        // not public (`api::get_son` gates it), so
                                        // this is not a state a visitor can see.
                                        // It exists because they can now reach it:
                                        // without a badge the page is identical to
                                        // a live son's, and the admin reviewing it
                                        // has no way to tell that what they are
                                        // looking at is not in the gallery.
                                        {(!s.is_public)
                                            .then(|| {
                                                view! {
                                                    <p class="mt-2 inline-flex items-center gap-1.5 rounded-full border border-danger/30 bg-danger/15 px-2.5 py-1 text-[0.78rem] font-medium text-danger">
                                                        <Ico icon=LuCircleAlert size=13/>
                                                        "Hidden \u{2014} not in the gallery"
                                                    </p>
                                                }
                                            })}

                                        // The avatar or icon belongs to the
                                        // name, so it sits in the same span at
                                        // a tighter gap -- at a flat gap-2 it
                                        // floated equidistant between the name
                                        // and the separator and read as its own
                                        // item. The separator is decorative and
                                        // aria-hidden: it carries no meaning a
                                        // screen reader announcing "middle dot"
                                        // would add.
                                        <div class="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-[0.8125rem] text-ink-3">
                                            <span class="inline-flex items-center gap-1.5">
                                                // The uploader's picture when
                                                // there is one. It has been on
                                                // the wire in `Uploader` all
                                                // along and was never drawn.
                                                // Explicit width/height so the
                                                // line cannot reflow when it
                                                // loads, and alt="" because the
                                                // name it labels is the very
                                                // next thing in this span.
                                                {match uploader_avatar {
                                                    Some(src) => {
                                                        view! {
                                                            <img
                                                                class="h-[18px] w-[18px] flex-none rounded-full object-cover"
                                                                src=src
                                                                alt=""
                                                                width="18"
                                                                height="18"
                                                                loading="lazy"
                                                                decoding="async"
                                                            />
                                                        }
                                                            .into_any()
                                                    }
                                                    None => view! { <Ico icon=LuUserRound size=14/> }.into_any(),
                                                }}
                                                {uploader_name}
                                            </span>
                                            <span class="text-line-strong" aria-hidden="true">"·"</span>
                                            // <time> so the machine-readable
                                            // date survives being shown in a
                                            // human format.
                                            <time datetime=iso_date.clone()>
                                                {pretty_date(&iso_date)}
                                            </time>
                                        </div>

                                        // A real surface, not a hairline with
                                        // glyphs adrift under it. Same border,
                                        // radius and fill as the figure directly
                                        // above, so the son and the things you
                                        // can do to it read as one object; the
                                        // rule that used to be here was doing
                                        // the work of a container without
                                        // looking like one.
                                        //
                                        // Hierarchy is positional: the primary
                                        // action holds the leading edge, the
                                        // three secondary ones group at the
                                        // trailing edge, and the primary is the
                                        // only labelled and counted control
                                        // among them.
                                        //
                                        // `flex-wrap` is structural, not
                                        // cosmetic. Without it a flex item that
                                        // grows -- the report panel, when it
                                        // opens -- shrinks its siblings instead
                                        // of moving to a new line, which
                                        // measured as the download and share
                                        // buttons squeezing from 36px to 29px.
                                        <div class="mt-4 flex flex-wrap items-center gap-x-1 gap-y-2 rounded-lg border border-line bg-surface p-1.5">
                                            <LikeButton
                                                id=s.id.clone()
                                                initial_count=s.likes
                                                initial_liked=s.liked_by_me
                                                prominent=true
                                            />
                                            // `ml-auto` puts this and everything
                                            // after it on the trailing edge.
                                            // Checked against style/tailwind.css
                                            // rather than assumed: `.icon-btn`
                                            // sets no margin, so this is not a
                                            // same-property fight with the
                                            // primitive.
                                            // `download` both asks the browser to
                                            // save rather than navigate *and* stops
                                            // leptos_router hijacking the click:
                                            // without it the router routed this
                                            // Axum-only path client-side, matched no
                                            // Leptos route, and the download button
                                            // rendered the 404 page.
                                            <a
                                                class="icon-btn ml-auto"
                                                download=""
                                                href=format!("/son/{}/download", s.slug)
                                                aria-label="Download"
                                                title="Download"
                                            >
                                                <Ico icon=LuDownload size=17/>
                                            </a>
                                            // The canonical path. ShareButton
                                            // resolves it against the live
                                            // origin, because absolute() is a
                                            // no-op in the wasm build.
                                            <ShareButton
                                                url=page_url.clone()
                                                title=s.title.clone()
                                            />
                                            // Wrapped because this control is
                                            // three different shapes: a lone
                                            // <button> when closed, a whole
                                            // panel when open, a <p> once it has
                                            // been sent. `:has(fieldset)` is
                                            // true only for the open panel --
                                            // the reason picker is the only
                                            // fieldset on this page -- and gives
                                            // the wrapper flex-basis 100%, so
                                            // the panel wraps to its own line at
                                            // the bar's full width instead of
                                            // being wedged into 208px beside the
                                            // icons.
                                            //
                                            // This reads report.rs's markup from
                                            // the outside. If that form ever
                                            // stops using a fieldset the panel
                                            // silently goes back to being
                                            // squeezed in, with no compile error
                                            // and nothing visibly wrong until
                                            // someone opens it on a phone.
                                            <div class="has-[fieldset]:basis-full has-[p]:basis-full">
                                                <ReportForm son_id=s.id.clone()/>
                                            </div>
                                            // Last in the bar, past report: it
                                            // is the only irreversible control
                                            // here, so it sits furthest from
                                            // the one people actually came to
                                            // press.
                                            <AdminDelete id=s.id.clone() slug=s.slug.clone()/>
                                        </div>
                                    </div>
                                </article>

                                <MoreSons exclude=s.id.clone()/>
                            }
                                .into_any()
                        }
                    })
            }}
        </Transition>
    }
}

#[cfg(test)]
mod tests {
    use super::pretty_date;

    #[test]
    fn formats_an_iso_date() {
        assert_eq!(pretty_date("2026-08-12"), "Aug 12, 2026");
    }

    #[test]
    fn drops_the_leading_zero_from_the_day() {
        // "Aug 05" reads like a serial number; the month name already tells a
        // reader this is a date, so the padding earns nothing.
        assert_eq!(pretty_date("2026-08-05"), "Aug 5, 2026");
    }

    #[test]
    fn handles_both_ends_of_the_year() {
        assert_eq!(pretty_date("2024-01-01"), "Jan 1, 2024");
        assert_eq!(pretty_date("2024-12-31"), "Dec 31, 2024");
    }

    /// Every rejected shape falls back to the input untouched. A wrong date
    /// shown confidently is worse than an ugly one, so none of these may
    /// produce a month name.
    #[test]
    fn anything_unexpected_falls_back_to_the_input() {
        for raw in [
            "",
            "not-a-date",
            "2026-13-01", // month past December
            "2026-00-01", // month 0, the wrapping_sub underflow case
            "26-08-12",   // two-digit year
            "2026-08",    // missing day
            "2026-08-12-01",
            "2026-aa-12",
        ] {
            assert_eq!(
                pretty_date(raw),
                raw,
                "expected {raw:?} to pass through unchanged"
            );
        }
    }
}
