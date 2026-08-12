use crate::app::SitePreview;
use leptos::prelude::*;
use leptos_meta::{Link, Meta, Title};
use leptos_router::components::A;

use crate::components::icon::{Ico, LuCheck, LuCircleAlert, LuCloudUpload, LuX};
use crate::models::{Step, UploadResult};
use crate::seo::absolute;

// `Progress` is deserialized only inside the poll loop, which is hydrate-only.
// The signal below holds a bare `Step`, so nothing in the view names this type
// any more -- left in the shared import it becomes an unused import under
// `--features ssr` and `clippy -D warnings` fails the whole gate.
#[cfg(feature = "hydrate")]
use crate::models::Progress;

/// How often the browser asks the server where the upload has got to. Fast
/// enough that each step is visibly acknowledged, slow enough that a minute of
/// processing is ~75 requests rather than thousands.
#[cfg(feature = "hydrate")]
const POLL_MS: u32 = 800;

/// Deliberately a hand-restated copy of `storage::MAX_UPLOAD_BYTES`, not a
/// reference to it: `crate::storage` is `#[cfg(feature = "ssr")]` and cannot be
/// named from a component that also compiles to wasm. If that constant ever
/// moves, this and the "12 MB max" line in the prompt drift in silence.
///
/// Worth the duplication because the alternative is the worst outcome this page
/// has: a 13 MB file uploads in full, takes the best part of a minute over a
/// phone connection, and only then fails in the decoder.
#[cfg(feature = "hydrate")]
const MAX_BYTES: f64 = 12.0 * 1024.0 * 1024.0;

/// Mirrors the `accept` attribute on the input. It exists twice because
/// `accept` filters the file picker and does precisely nothing for a dropped
/// file -- a drop is the only reason this list is checked in Rust at all.
#[cfg(feature = "hydrate")]
const ACCEPTED: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// The box every outcome is drawn in. A `const` rather than four copies of the
/// same literal; the Tailwind scanner reads this file as raw text, so a `const`
/// is exactly as visible to it as a class attribute would be.
const OUTCOME_SHELL: &str =
    "mt-4 flex flex-col items-center gap-3 rounded-lg border border-line bg-surface p-4 text-center";

/// The six steps the server reports, collapsed to the three facts a visitor can
/// do anything with.
///
/// The server still reports six because six is what it actually runs, and the
/// `/judge`-`/square` split still earns its keep -- it lets the bar below weight
/// the long stage correctly, and it lets a refusal come back in ~3s instead of
/// after an 80s generation that was going to be thrown away. But none of that is
/// the visitor's business. From where they are standing there are three things
/// to know: the file is leaving their device, someone has it, it is being put
/// away. Reciting "Fingerprinting" at them is the app talking about itself.
fn phase(step: Step) -> &'static str {
    match step {
        Step::Receiving => "Uploading",
        Step::Fingerprinting | Step::Scanning | Step::Regenerating | Step::Cropping => "Working",
        Step::Storing => "Saving",
    }
}

/// Where a step sits on the bar: `(start, ceiling, tau)`, as fractions of one.
///
/// The six steps are not six equal sixths of a minute. `Regenerating` is Gemini
/// drawing a 1024px image and is measured at 30-80s (`gemini.rs` puts it at ~50s
/// locally and ~85s through production); `Cropping` is a resize that is finished
/// before the next poll arrives. A bar that gave each step 1/6 would sit at 67%
/// for the best part of a minute and then sprint -- which reads first as frozen
/// and then as a lie.
///
/// Within a band the fill is `start + (ceiling - start) * t / (t + tau)`, where
/// `t` is the number of polls seen since this step began. Three properties make
/// that honest, and each of them is easy to void by accident later:
///
/// - **Monotonic.** Every band's ceiling is the next band's floor, and the
///   hyperbolic term never reaches 1, so the bar can never walk backwards. That
///   also covers the case where `GEMINI_URL` is unset and the stream jumps
///   straight from `Fingerprinting` to `Cropping`: the bar leaps to 88%, which
///   is correct -- the upload really is nearly done. Reordering the pipeline in
///   `upload_route.rs` without reordering this function breaks it.
/// - **Always moving.** `t` grows on every poll even when the server reports the
///   same step for a minute, so the bar never looks stuck.
/// - **Never full before `Done`.** The last ceiling is 0.99 and is approached,
///   never reached. A bar that completes and then waits is the classic lie.
///
/// `tau` is in polls, not seconds, because `POLL_MS` is hydrate-only and this
/// function compiles under both feature sets. At 800ms a poll, tau of 4 is
/// ~3.2s, 12 is ~9.6s, 6 is ~4.8s. Counting polls also errs conservative: a
/// backgrounded tab throttles timers to roughly 1Hz, so `t` under-counts and the
/// bar sits behind reality rather than ahead of it.
fn band(step: Step) -> (f64, f64, f64) {
    match step {
        Step::Receiving => (0.00, 0.08, 4.0),
        Step::Fingerprinting => (0.08, 0.14, 2.0),
        Step::Scanning => (0.14, 0.22, 4.0),
        Step::Regenerating => (0.22, 0.88, 12.0),
        Step::Cropping => (0.88, 0.91, 1.0),
        Step::Storing => (0.91, 0.99, 6.0),
    }
}

/// The free upload page.
///
/// Posts multipart directly to `/api/upload` rather than through a server
/// function: server fns would have to base64 the file through a serde payload,
/// inflating it by a third for no benefit.
///
/// No sign-in anywhere on this page, on purpose. `upload_route::upload` falls
/// back to an anonymous uploader when the session cookie is missing, so an
/// account only ever adds attribution -- it is never a gate.
#[component]
pub fn Upload() -> impl IntoView {
    let (preview, set_preview) = signal(Option::<String>::None);
    let (filename, set_filename) = signal(Option::<String>::None);
    let (size_text, set_size_text) = signal(Option::<String>::None);
    let (busy, set_busy) = signal(false);
    let (result, set_result) = signal(Option::<UploadResult>::None);
    // Where the pipeline is, as last reported, and how many polls have come back
    // saying so. Together they are the whole input to the progress bar. A `Step`
    // rather than a `Progress` because the three terminal states already move
    // into `result` -- keeping them here too would mean two places to look for
    // "did this finish".
    let (step, set_step) = signal(Option::<Step>::None);
    let (polls, set_polls) = signal(0u32);
    // Whether the preview image has decoded. Drives its fade-in: the element
    // mounts transparent and this flips on the `load` event, which for an object
    // URL is always at least a task after mount, so the transparent style has
    // been computed by then and the transition actually runs. An event handler
    // is not render, so writing a signal here is not the hydration-killing kind.
    let (loaded, set_loaded) = signal(false);
    // Whether a file is currently being dragged over the drop zone. Purely
    // visual, but without it there is no feedback that the page will accept it.
    let (dragging, set_dragging) = signal(false);

    let file_input: NodeRef<leptos::html::Input> = NodeRef::new();
    let title_input: NodeRef<leptos::html::Input> = NodeRef::new();

    // Every setter above is written only from the hydrate-only handlers below,
    // so the server build sees them as unused. The form is inert until wasm
    // takes over, which is expected — not a missing code path.
    #[cfg(not(feature = "hydrate"))]
    let _ = (
        set_preview,
        set_filename,
        set_size_text,
        set_busy,
        set_result,
        set_step,
        set_polls,
        set_loaded,
        set_dragging,
        title_input,
    );

    // Back to "nothing picked", from either the Remove button or a file this
    // page refuses. Clearing the input matters in both cases: a drop assigns the
    // FileList before anything is validated, so a rejected drop would otherwise
    // leave the zone showing the previous image while the input still held the
    // bad file, and submit reads the input.
    //
    // `set_value("")` is the specified way to empty a file input; `set_files`
    // with an empty list is not reliably honoured.
    #[cfg(feature = "hydrate")]
    let clear_picked = move || {
        if let Some(old) = preview.get_untracked() {
            let _ = web_sys::Url::revoke_object_url(&old);
        }
        set_preview.set(None);
        set_filename.set(None);
        set_size_text.set(None);
        set_loaded.set(false);
        if let Some(input) = file_input.get() {
            input.set_value("");
        }
    };

    // Shared by the file picker and by a drop, so both routes produce the same
    // preview and the same cleared-out previous result. Only compiled for the
    // browser: `web_sys::File` is a hydrate-only dependency, and there is no
    // file picker on the server.
    #[cfg(feature = "hydrate")]
    let show_preview = move |file: web_sys::File| {
        // File is a Blob subclass in the DOM; web-sys models that as AsRef,
        // so no cast is needed.
        let blob: &web_sys::Blob = file.as_ref();

        if blob.size() > MAX_BYTES {
            clear_picked();
            set_result.set(Some(UploadResult::Error {
                message: "That file is over 12 MB.".into(),
            }));
            return;
        }
        // Fails open on an empty type. Several platforms hand over a `File` with
        // no MIME type at all -- a drop out of some file managers, and anything
        // with an extension the OS does not recognise -- and refusing those
        // would turn away valid images with nothing the visitor could act on.
        // The server decodes by sniffing the bytes anyway, so this check is a
        // courtesy, not the boundary.
        let mime = blob.type_();
        if !mime.is_empty() && !ACCEPTED.contains(&mime.as_str()) {
            clear_picked();
            set_result.set(Some(UploadResult::Error {
                message: "That needs to be a PNG, JPG, WEBP or GIF.".into(),
            }));
            return;
        }

        // Revoke the URL this one replaces, not the one just created: every
        // picked file used to leak an object URL for the life of the page, and
        // revoking the new one instead simply blanks the preview.
        if let Some(old) = preview.get_untracked() {
            let _ = web_sys::Url::revoke_object_url(&old);
        }
        // Before `set_preview`, so the freshly mounted image is built from a
        // `loaded` that is already false and has something to transition from.
        set_loaded.set(false);
        if let Ok(url) = web_sys::Url::create_object_url_with_blob(blob) {
            set_preview.set(Some(url));
        }
        set_filename.set(Some(file.name()));
        let bytes = blob.size();
        set_size_text.set(Some(if bytes >= 1024.0 * 1024.0 {
            format!("{:.1} MB", bytes / (1024.0 * 1024.0))
        } else {
            format!("{:.0} KB", (bytes / 1024.0).max(1.0))
        }));
        set_result.set(None);
        set_step.set(None);
        set_polls.set(0);
    };

    let on_file_change = move |_| {
        #[cfg(feature = "hydrate")]
        {
            if let Some(file) = file_input
                .get()
                .and_then(|i| i.files())
                .and_then(|f| f.get(0))
            {
                show_preview(file);
            }
        }
    };

    let remove = move |_ev: leptos::ev::MouseEvent| {
        #[cfg(feature = "hydrate")]
        {
            clear_picked();
            set_result.set(None);
        }
    };

    // Drag and drop. The handlers sit on the wrapper around the drop zone, not
    // on the label itself, because the Remove button overlays the zone as a
    // sibling of that label: a file dropped on that corner would land on an
    // element with no preventDefault on dragover, and the browser would navigate
    // to the image instead -- exactly the failure the comment below was written
    // for. The wrapper is the one element every drop is guaranteed to reach.
    //
    // dragover MUST preventDefault on every event, not just once: the browser's
    // default for a dragged file is "navigate to it", and it re-checks on each
    // dragover. Without it the drop silently opens the image instead, which is
    // exactly how this was broken.
    let on_drag_over = move |ev: leptos::ev::DragEvent| {
        ev.prevent_default();
        set_dragging.set(true);
    };
    let on_drag_leave = move |ev: leptos::ev::DragEvent| {
        ev.prevent_default();
        set_dragging.set(false);
    };
    let on_drop = move |ev: leptos::ev::DragEvent| {
        ev.prevent_default();
        set_dragging.set(false);
        #[cfg(feature = "hydrate")]
        {
            // The file input is disabled while an upload runs, which stops the
            // click route, but a drop is delivered to the wrapper regardless.
            // Swapping the file out from under a running job would leave the
            // preview and the job describing two different images.
            if busy.get_untracked() {
                return;
            }
            let Some(dt) = ev.data_transfer() else { return };
            let Some(files) = dt.files() else { return };
            let Some(file) = files.get(0) else { return };
            // Assign the dropped FileList onto the hidden input, rather than
            // keeping the File in a signal: submit reads the input, so this
            // keeps one source of truth and means a drop and a click are
            // indistinguishable from there on.
            if let Some(input) = file_input.get() {
                input.set_files(Some(&files));
            }
            show_preview(file);
        }
    };

    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();

        #[cfg(feature = "hydrate")]
        {
            let Some(input) = file_input.get() else {
                return;
            };
            let Some(file) = input.files().and_then(|f| f.get(0)) else {
                set_result.set(Some(UploadResult::Error {
                    message: "Pick a son first.".into(),
                }));
                return;
            };

            let title = title_input.get().map(|t| t.value()).unwrap_or_default();

            set_busy.set(true);
            set_result.set(None);
            set_step.set(Some(Step::Receiving));
            set_polls.set(0);

            leptos::task::spawn_local(async move {
                let form = web_sys::FormData::new().expect("FormData unavailable");
                let _ = form.append_with_blob_and_filename("son", &file, &file.name());
                let _ = form.append_with_str("title", &title);

                let queued =
                    match gloo_net::http::Request::post("/api/upload")
                        .body(form)
                        .expect("FormData is a valid body")
                        .send()
                        .await
                    {
                        Ok(resp) => resp.json::<UploadResult>().await.unwrap_or_else(|e| {
                            UploadResult::Error {
                                message: format!("unexpected reply from the server: {e}"),
                            }
                        }),
                        Err(e) => UploadResult::Error {
                            message: format!("could not reach the server: {e}"),
                        },
                    };

                // Anything but a job id is already final -- a malformed request,
                // or the server refusing before it started work.
                let UploadResult::Queued { job } = queued else {
                    set_busy.set(false);
                    set_step.set(None);
                    set_result.set(Some(queued));
                    return;
                };

                // How long the server has been saying the same thing, counted in
                // polls. Two plain locals rather than reading the signals back:
                // the loop is the only writer, so it already knows.
                let mut current = Step::Receiving;
                let mut seen = 0u32;

                // Poll until the job reaches a terminal state. A failed request
                // mid-poll is not terminal -- the server may just have been busy
                // -- so it waits and asks again rather than reporting failure.
                // A job the server has forgotten (a restart drops every job in
                // flight, they live in memory) comes back as Failed, so the
                // browser is told rather than left polling a ghost.
                loop {
                    gloo_timers::future::TimeoutFuture::new(POLL_MS).await;

                    let fetched =
                        gloo_net::http::Request::get(&format!("/api/upload/status/{job}"))
                            .send()
                            .await;

                    let Ok(resp) = fetched else { continue };
                    let Ok(p) = resp.json::<Progress>().await else {
                        continue;
                    };

                    match p {
                        Progress::Running { step: s } => {
                            if s == current {
                                seen += 1;
                            } else {
                                current = s;
                                seen = 0;
                            }
                            set_step.set(Some(s));
                            set_polls.set(seen);
                        }
                        Progress::Done { son } => {
                            set_busy.set(false);
                            set_step.set(None);
                            set_result.set(Some(UploadResult::Ok { son: *son }));
                            return;
                        }
                        Progress::Rejected { reason } => {
                            set_busy.set(false);
                            set_step.set(None);
                            set_result.set(Some(UploadResult::Rejected { reason }));
                            return;
                        }
                        Progress::Failed { message } => {
                            set_busy.set(false);
                            set_step.set(None);
                            set_result.set(Some(UploadResult::Error { message }));
                            return;
                        }
                    }
                }
            });
        }
    };

    // The bar's fill, already in percent and already floored. One closure so the
    // width and the announced value cannot disagree.
    //
    // The floor is 2%: a zero-width bar is indistinguishable from no bar at all,
    // and "we have started" is true from the moment the POST leaves.
    let shown = move || {
        let raw = match step.get() {
            Some(s) => {
                let (start, ceiling, tau) = band(s);
                let t = polls.get() as f64;
                start + (ceiling - start) * t / (t + tau)
            }
            None => 0.0,
        };
        (raw * 100.0).max(2.0)
    };

    // Three whole class strings, never a shared base with a state layered on
    // top. The version this replaces put `bg-surface` in the base and appended
    // the drag tint in the branch: both landed, both were single-class selectors
    // at equal specificity, and the later rule in the stylesheet won -- so the
    // drag-hover tint had never once rendered, and it looked fine because the
    // border still changed. The base below holds no background, no border colour
    // and no border style, which is the whole fix.
    let zone = move || {
        let base = "relative grid min-h-[260px] min-w-0 cursor-pointer grid-cols-[minmax(0,1fr)] place-items-center overflow-hidden rounded-lg border-2 p-5 text-center transition-colors duration-200 ease-out has-[:focus-visible]:outline has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-accent lg:min-h-[320px]";
        let state = if dragging.get() {
            "border-solid border-accent bg-accent-soft"
        } else if preview.get().is_some() {
            "border-solid border-line-strong bg-surface"
        } else {
            "border-dashed border-line bg-surface hover:border-accent-border has-[:focus-visible]:border-accent"
        };
        format!("{base} {state}")
    };

    view! {
        <Title text="Contribute — son collection"/>
        <SitePreview/>
        <Meta
            name="description"
            content="Contribute a son to the collection."
        />
        <Link rel="canonical" href=absolute("/upload")/>

        <section class="mx-auto max-w-[620px] pt-5">
            // Just the task name. The paragraph that used to sit here narrated
            // the service ("Free, no account. Everything is checked before it
            // goes live…") -- none of which a visitor needs in order to pick a
            // file, so it is gone rather than reworded.
            <h1 class="m-0 mb-4 text-[1.375rem] font-bold tracking-tight lg:mb-6 lg:text-[1.75rem]">"Contribute"</h1>

            // `grid-cols-[minmax(0,1fr)]`, not a bare `grid`: an implicit track
            // is sized by its widest item's min-content, and the drop zone's
            // min-content came to 388px at a 393px viewport. That widened the
            // single track and dragged the file input and submit button out
            // with it, so the whole form scrolled sideways on a phone. The
            // explicit `minmax(0, ...)` lets the track shrink below min-content.
            <form on:submit=submit aria-busy=move || busy.get().to_string() class="grid grid-cols-[minmax(0,1fr)] gap-3.5">
                // The wrapper, not the label, owns the drag handlers -- see
                // `on_drag_over`. It is also the positioning context for the
                // three things that sit over the zone: the Remove button, the
                // busy overlay, and the preview itself.
                <div
                    class="relative"
                    on:dragover=on_drag_over
                    on:dragenter=on_drag_over
                    on:dragleave=on_drag_leave
                    on:drop=on_drop
                >
                    // `has-[:focus-visible]` puts the focus ring on the drop
                    // zone, because the input it belongs to is visually hidden
                    // inside it -- without it a keyboard user tabbing here would
                    // see nothing at all change. `overflow-hidden` clips the
                    // preview and the filename strip to the rounded border; it
                    // does not clip the element's own outline, so the focus ring
                    // still draws in full.
                    //
                    // The height is pinned rather than grown into. Before this,
                    // picking a file took the zone from 260px to as much as
                    // 420px and shoved the title field, the button and every
                    // result box down the page by around 160px, which is the
                    // single most jarring thing this page did.
                    <label class=zone>
                        // `sr-only`, not a styled-down native control: a file
                        // input's width comes from its own "Choose file / no file
                        // selected" chrome, which is 344px in Chrome and refuses to
                        // shrink -- `max-width` does not clamp it, so it pushed the
                        // page sideways at 360px and narrower. The label around it
                        // already renders the whole drop zone, including the
                        // "Choose a file" prompt the native button duplicated.
                        // Hidden this way it stays focusable and screen-reader
                        // reachable, unlike `display: none`.
                        //
                        // Disabled while a job runs so the zone cannot open a
                        // picker mid-upload. Nothing is submitted natively -- the
                        // multipart body is assembled by hand -- so disabling it
                        // costs nothing at submit time.
                        <input
                            class="sr-only"
                            type="file"
                            accept="image/png,image/jpeg,image/webp,image/gif"
                            disabled=move || busy.get()
                            node_ref=file_input
                            on:change=on_file_change
                        />
                        // Permanently mounted and faded, never unmounted. Two
                        // reasons, both real: this text is the label's content
                        // and therefore the sr-only input's only accessible
                        // name, so swapping it out for the preview left a
                        // nameless control; and an element that stays put is the
                        // only thing a CSS transition can animate, which is the
                        // whole of the motion budget here (no custom keyframes
                        // exist in this project's config).
                        //
                        // `pointer-events-none` is load-bearing rather than
                        // tidy: dragleave bubbles up from children, so dragging
                        // across this text used to fire dragleave on the zone
                        // and flicker the highlight off -- masked only by the
                        // next dragover setting it straight back. Making every
                        // inner layer transparent to hit-testing removes the
                        // flicker outright and lets clicks fall through to the
                        // label.
                        <span class=move || {
                            if preview.get().is_some() {
                                "pointer-events-none grid gap-1 text-center text-ink-2 transition-opacity duration-200 ease-out opacity-0"
                            } else {
                                "pointer-events-none grid gap-1 text-center text-ink-2 transition-opacity duration-200 ease-out opacity-100"
                            }
                        }>
                            <span class="mx-auto inline-flex text-ink-3">
                                <Ico icon=LuCloudUpload size=26/>
                            </span>
                            <strong>
                                // "Drop a file" is an instruction a
                                // phone cannot follow -- there is
                                // nothing to drag with -- so touch gets
                                // the prompt that matches what it can
                                // actually do.
                                //
                                // Chosen by `pointer:fine` rather than a
                                // width breakpoint, because the question
                                // is whether this device has a pointer,
                                // not how wide it is: a 1400px touch
                                // screen still cannot drag, and a 700px
                                // window with a mouse still can. Both
                                // strings are rendered and CSS hides
                                // one, so nothing branches on the
                                // viewport in Rust -- the server has no
                                // window, and disagreeing with the
                                // client there is what killed the wasm
                                // module once already.
                                {move || {
                                    if dragging.get() {
                                        view! { <span>"Drop it"</span> }.into_any()
                                    } else {
                                        view! {
                                            <span>
                                                <span class="[@media(pointer:fine)]:hidden">
                                                    "Choose a file"
                                                </span>
                                                <span class="hidden [@media(pointer:fine)]:inline">
                                                    "Drop a file, or choose one"
                                                </span>
                                            </span>
                                        }
                                            .into_any()
                                    }
                                }}
                            </strong>
                            // Kept: the format/size line prevents a
                            // failed upload, and these values mirror
                            // MAX_UPLOAD_BYTES and the accept list
                            // rather than being restated by hand.
                            <small>"PNG, JPG, WEBP, GIF · 12 MB max"</small>
                        </span>

                        // A mapping closure, not a `<Show>` reading the signal
                        // back out with `unwrap_or_default`: that form can emit
                        // `src=""` on the frame it unmounts, which some browsers
                        // resolve as a fresh request for the current document.
                        // Reachable now that Remove can put `preview` back to
                        // None. This closure captures a real String and cannot
                        // produce an empty src.
                        //
                        // Absolutely positioned because an in-flow image is what
                        // used to resize the zone. The class is its own reactive
                        // closure so only the attribute updates on load -- if
                        // the opacity were read in the outer closure the whole
                        // element would be rebuilt on every load event, which
                        // both kills the fade and re-fires `load` forever.
                        {move || {
                            preview
                                .get()
                                .map(|url| {
                                    view! {
                                        <img
                                            class=move || {
                                                if loaded.get() {
                                                    "pointer-events-none absolute inset-0 h-full w-full rounded object-contain p-3 transition-opacity duration-300 ease-out opacity-100"
                                                } else {
                                                    "pointer-events-none absolute inset-0 h-full w-full rounded object-contain p-3 transition-opacity duration-300 ease-out opacity-0"
                                                }
                                            }
                                            src=url
                                            alt=""
                                            on:load=move |_| set_loaded.set(true)
                                        />
                                    }
                                })
                        }}

                        // The filename, pinned inside the zone rather than sat
                        // in the form below it -- it was the second thing that
                        // shifted the whole page when a file was picked. Same
                        // gradient treatment the gallery cards already use, so
                        // it stays readable over a white photograph. Hidden
                        // while busy: it shares this edge with the progress bar
                        // and the two never need to be seen at once.
                        {move || {
                            (preview.get().is_some() && !busy.get())
                                .then(|| {
                                    view! {
                                        <span class="pointer-events-none absolute inset-x-0 bottom-0 flex items-center gap-2 bg-gradient-to-t from-black/90 via-black/60 to-transparent px-3 pb-2.5 pt-10 text-left text-[0.8125rem] text-ink-2">
                                            <span class="min-w-0 truncate">
                                                {move || filename.get().unwrap_or_default()}
                                            </span>
                                            <span class="flex-none tabular-nums text-ink-3">
                                                {move || size_text.get().unwrap_or_default()}
                                            </span>
                                        </span>
                                    }
                                })
                        }}
                    </label>

                    // There was no way to un-pick a file at all before this.
                    //
                    // Not the `.icon-btn` primitive: that sets a transparent
                    // border, a transparent background, a muted text colour and
                    // a surface hover, and this control sits on top of an
                    // arbitrary photograph, so it needs all four different.
                    // Adding them as utilities would put four equal-specificity
                    // pairs in play and hand the result to stylesheet order.
                    // One standalone string instead, with the same on-image
                    // treatment the small like button uses.
                    {move || {
                        (preview.get().is_some() && !busy.get())
                            .then(|| {
                                view! {
                                    <button
                                        type="button"
                                        class="absolute right-2 top-2 inline-flex h-9 w-9 items-center justify-center rounded-full border border-white/25 bg-black/55 text-ink backdrop-blur-sm transition-colors hover:border-danger hover:text-danger"
                                        aria-label="Remove"
                                        on:click=remove
                                    >
                                        <Ico icon=LuX size=16/>
                                    </button>
                                }
                            })
                    }}

                    // The busy overlay. Mounted from first paint and faded by an
                    // opacity swap rather than mounted on demand, for two
                    // reasons: a scrim that pops in reads as a glitch, and the
                    // live region inside it has to already exist when its text
                    // arrives -- a region inserted at the same moment as its
                    // contents announces nothing at all.
                    <div class=move || {
                        if busy.get() {
                            "pointer-events-none absolute inset-0 overflow-hidden rounded-lg bg-black/60 opacity-100 backdrop-blur-sm transition-opacity duration-300 ease-out"
                        } else {
                            "pointer-events-none absolute inset-0 overflow-hidden rounded-lg bg-black/60 opacity-0 backdrop-blur-sm transition-opacity duration-300 ease-out"
                        }
                    }>
                        // One word, and it is empty when idle so nothing is
                        // announced until there is something to announce. The
                        // whole of the narration this page used to do: six named
                        // pipeline stages, each with its own row and its own
                        // tick. It changes three times in fifty seconds, which
                        // is about the rate a spoken announcement can be useful
                        // at -- and it is exactly why the percentage is kept out
                        // of this element and left on the bar as an attribute.
                        <p
                            class="m-0 flex h-full w-full items-center justify-center text-[0.9375rem] font-semibold text-ink"
                            aria-live="polite"
                        >
                            {move || step.get().map(phase).unwrap_or_default()}
                        </p>

                        // The bar is the only computed visual on this page and
                        // its width is an inline style, never a class: a
                        // `w-[37%]` assembled at runtime is invisible to the
                        // Tailwind scanner and would generate no rule at all.
                        //
                        // The 700ms transition against an 800ms poll makes the
                        // fill glide continuously instead of stepping once a
                        // second. The target is always the honest number; the
                        // glide is interpolation toward it, never past it. The
                        // global reduced-motion rule already turns this into a
                        // snap, so there are no motion variants layered here.
                        <Show when=move || busy.get()>
                            <div
                                role="progressbar"
                                aria-label="Upload progress"
                                aria-valuemin="0"
                                aria-valuemax="100"
                                aria-valuenow=move || (shown().round() as i32).to_string()
                                class="absolute inset-x-0 bottom-0 h-1 overflow-hidden bg-white/15"
                            >
                                <div
                                    class="h-full bg-accent transition-[width] duration-700 ease-out"
                                    style:width=move || format!("{:.1}%", shown())
                                />
                            </div>
                        </Show>
                    </div>
                </div>

                // Labelled, not just placeholder'd: a placeholder stops being an
                // accessible name the moment anything is typed into the field.
                // No utilities on top of `.field` -- it already owns the width,
                // border, background, padding, text size and placeholder colour.
                <input
                    class="field"
                    type="text"
                    placeholder="Name this son"
                    aria-label="Name this son"
                    maxlength="80"
                    node_ref=title_input
                />

                // The label stays "Upload" while a job runs. The overlay above
                // is already saying what is happening, and two controls
                // narrating the same state is precisely the blabber this page
                // was cut down to remove. `.btn` carries the disabled styling.
                <button class="btn" type="submit" disabled=move || busy.get()>
                    "Upload"
                </button>
            </form>

            {move || {
                result
                    .get()
                    .map(|r| match r {
                        // Only ever seen for the blink between the POST
                        // returning and the first poll; rendered as nothing
                        // rather than as a state of its own.
                        UploadResult::Queued { .. } => ().into_any(),
                        // A held son (is_public false) is saved but not in the
                        // gallery, because screening could not run. Saying
                        // "Uploaded" for it would be a lie by omission -- the
                        // uploader would go looking for it and not find it.
                        //
                        // This is the outage path and it keeps its own words. A
                        // son that was refused and a son that could not be
                        // checked are different events, and rendering them the
                        // same box is the one collapse this project says must
                        // never happen -- which is easy to do by accident
                        // exactly here, while sharing one component between all
                        // four outcomes.
                        UploadResult::Ok { son } if !son.is_public => {
                            view! {
                                <Outcome
                                    icon=LuCircleAlert
                                    tone="inline-flex h-9 w-9 items-center justify-center rounded-full bg-surface-raised text-accent"
                                    headline="Saved, waiting on review"
                                    note="We couldn't check this one automatically, so it won't appear until someone looks at it."
                                />
                            }
                                .into_any()
                        }
                        UploadResult::Ok { son } => {
                            view! {
                                <Outcome
                                    icon=LuCheck
                                    tone="inline-flex h-9 w-9 items-center justify-center rounded-full bg-surface-raised text-ok"
                                    headline="Uploaded"
                                    slug=son.slug
                                />
                            }
                                .into_any()
                        }
                        // `reason` is this project's own wording, written in
                        // `Verdict::acceptable`, never anything the model
                        // generated. Nothing it says reaches a visitor.
                        UploadResult::Rejected { reason } => {
                            view! {
                                <Outcome
                                    icon=LuCircleAlert
                                    tone="inline-flex h-9 w-9 items-center justify-center rounded-full bg-surface-raised text-danger"
                                    headline=reason
                                />
                            }
                                .into_any()
                        }
                        UploadResult::Error { message } => {
                            view! {
                                <Outcome
                                    icon=LuCircleAlert
                                    tone="inline-flex h-9 w-9 items-center justify-center rounded-full bg-surface-raised text-danger"
                                    headline=message
                                />
                            }
                                .into_any()
                        }
                    })
            }}
        </section>
    }
}

/// One result box. Four near-identical blocks lived here before, and the risk
/// with that is not the duplication itself but that they drifted.
///
/// `tone` is the whole class string for the icon medallion, passed as a literal
/// from each call site rather than crossed with a colour modifier here: the
/// scanner reads complete literals, and a base class plus a colour utility is
/// the equal-specificity coin flip this codebase keeps out of its stylesheet.
///
/// No `children` prop. Nothing else in this project uses one, and every outcome
/// is the same three optional pieces.
#[component]
fn Outcome(
    icon: icondata_core::Icon,
    tone: &'static str,
    #[prop(into)] headline: String,
    /// Second line. Only the held-son outcome has one, and it is the reason
    /// that outcome cannot share copy with a refusal.
    #[prop(optional, into)]
    note: Option<String>,
    /// Present only when there is a published son to go and look at.
    #[prop(optional, into)]
    slug: Option<String>,
) -> impl IntoView {
    view! {
        <div class=OUTCOME_SHELL>
            <span class=tone>
                <Ico icon=icon size=18/>
            </span>
            <p class="m-0 text-[0.9375rem] font-semibold text-ink">{headline}</p>
            {note.map(|n| view! { <p class="m-0 text-[0.85rem] text-ink-2">{n}</p> })}
            // Built from the slug, never the id: every link on this site is, and
            // `/son/:slug` is what the router matches.
            {slug.map(|s| view! { <A href=format!("/son/{s}") attr:class="btn">"View son"</A> })}
        </div>
    }
}

/// The progress bar's honesty properties, asserted rather than believed.
///
/// They are all one edit away from being silently void -- reordering the
/// pipeline in `upload_route.rs`, shaving a tau, nudging a ceiling to 1.0 -- and
/// none of them fail loudly. They fail as a bar that goes backwards, or sticks,
/// or sits full while the visitor waits, which is exactly the class of thing
/// nobody notices until it is in production.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_tile_the_bar_without_gaps_or_overlap() {
        let mut floor = 0.0;
        for step in Step::ALL {
            let (start, ceiling, tau) = band(step);
            assert_eq!(start, floor, "{step:?} does not start where the last ended");
            assert!(ceiling > start, "{step:?} has no room to move");
            assert!(tau > 0.0, "{step:?} would divide by zero at the first poll");
            floor = ceiling;
        }
        // The last ceiling is approached, never reached. A bar that fills and
        // then waits is the classic lie, and `Done` is the only thing allowed
        // to end this.
        assert!(
            floor < 1.0,
            "the pipeline can reach 100% before it finishes"
        );
    }

    #[test]
    fn the_fill_always_moves_and_never_leaves_its_band() {
        for step in Step::ALL {
            let (start, ceiling, tau) = band(step);
            let at = |t: u32| start + (ceiling - start) * f64::from(t) / (f64::from(t) + tau);
            assert_eq!(at(0), start);
            // Strictly increasing for as long as any real job could run --
            // 10,000 polls is over two hours at 800ms, well past the point the
            // server would have given up.
            for t in 1..10_000u32 {
                assert!(at(t) > at(t - 1), "{step:?} stalled at poll {t}");
                assert!(at(t) < ceiling, "{step:?} overran its band at poll {t}");
            }
        }
    }

    #[test]
    fn every_step_reports_one_of_three_words() {
        for step in Step::ALL {
            assert!(
                ["Uploading", "Working", "Saving"].contains(&phase(step)),
                "{step:?} invented a fourth word"
            );
        }
    }
}
