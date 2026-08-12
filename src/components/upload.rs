use crate::app::SitePreview;
use leptos::prelude::*;
use leptos_meta::{Link, Meta, Title};
use leptos_router::components::A;

use crate::components::icon::{Ico, LuCheck, LuCircleAlert, LuCloudUpload};
use crate::models::{Progress, Step, UploadResult};
use crate::seo::absolute;

/// How often the browser asks the server where the upload has got to. Fast
/// enough that each step is visibly acknowledged, slow enough that a minute of
/// processing is ~75 requests rather than thousands.
#[cfg(feature = "hydrate")]
const POLL_MS: u32 = 800;

/// The free upload page.
///
/// Posts multipart directly to `/api/upload` rather than through a server
/// function: server fns would have to base64 the file through a serde payload,
/// inflating it by a third for no benefit.
#[component]
pub fn Upload() -> impl IntoView {
    let (preview, set_preview) = signal(Option::<String>::None);
    let (filename, set_filename) = signal(Option::<String>::None);
    let (busy, set_busy) = signal(false);
    let (result, set_result) = signal(Option::<UploadResult>::None);
    // Where the pipeline is, as last reported. Drives the step list below.
    let (progress, set_progress) = signal(Option::<Progress>::None);
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
        set_busy,
        set_result,
        set_progress,
        set_dragging,
        title_input,
    );

    // Shared by the file picker and by a drop, so both routes produce the same
    // preview and the same cleared-out previous result. Only compiled for the
    // browser: `web_sys::File` is a hydrate-only dependency, and there is no
    // file picker on the server.
    #[cfg(feature = "hydrate")]
    let show_preview = move |file: web_sys::File| {
        {
            // File is a Blob subclass in the DOM; web-sys models that as AsRef,
            // so no cast is needed.
            let blob: &web_sys::Blob = file.as_ref();
            if let Ok(url) = web_sys::Url::create_object_url_with_blob(blob) {
                set_preview.set(Some(url));
            }
            set_filename.set(Some(file.name()));
            set_result.set(None);
            set_progress.set(None);
        }
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

    // Drag and drop. The label is the drop target because it is the whole
    // visible drop zone; the file input inside it is sr-only and never receives
    // these events itself.
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
            set_progress.set(Some(Progress::Running {
                step: Step::Receiving,
            }));

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
                    set_progress.set(None);
                    set_result.set(Some(queued));
                    return;
                };

                // Poll until the job reaches a terminal state. A failed request
                // mid-poll is not terminal -- the server may just have been busy
                // -- so it waits and asks again rather than reporting failure.
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
                        Progress::Running { .. } => set_progress.set(Some(p)),
                        Progress::Done { son } => {
                            set_busy.set(false);
                            set_progress.set(None);
                            set_result.set(Some(UploadResult::Ok { son: *son }));
                            return;
                        }
                        Progress::Rejected { reason } => {
                            set_busy.set(false);
                            set_progress.set(None);
                            set_result.set(Some(UploadResult::Rejected { reason }));
                            return;
                        }
                        Progress::Failed { message } => {
                            set_busy.set(false);
                            set_progress.set(None);
                            set_result.set(Some(UploadResult::Error { message }));
                            return;
                        }
                    }
                }
            });
        }
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
            <form on:submit=submit class="grid grid-cols-[minmax(0,1fr)] gap-3.5">
                // `has-[:focus-visible]` puts the focus ring on the drop zone,
                // because the input it belongs to is visually hidden below --
                // without it a keyboard user tabbing here would see nothing at
                // all change.
                <label
                    class=move || {
                        let base = "grid min-h-[260px] min-w-0 cursor-pointer grid-cols-[minmax(0,1fr)] place-items-center rounded-lg border-2 border-dashed bg-surface p-5 text-center transition-colors has-[:focus-visible]:outline has-[:focus-visible]:outline-2 has-[:focus-visible]:outline-offset-2 has-[:focus-visible]:outline-accent";
                        if dragging.get() {
                            format!("{base} border-accent bg-accent-soft")
                        } else {
                            format!("{base} border-line hover:border-accent-border has-[:focus-visible]:border-accent")
                        }
                    }
                    on:dragover=on_drag_over
                    on:dragenter=on_drag_over
                    on:dragleave=on_drag_leave
                    on:drop=on_drop
                >
                    // `sr-only`, not a styled-down native control: a file
                    // input's width comes from its own "Choose file / no file
                    // selected" chrome, which is 344px in Chrome and refuses to
                    // shrink -- `max-width` does not clamp it, so it pushed the
                    // page sideways at 360px and narrower. The label around it
                    // already renders the whole drop zone, including the
                    // "Choose a file" prompt the native button duplicated.
                    // Hidden this way it stays focusable and screen-reader
                    // reachable, unlike `display: none`.
                    <input
                        class="sr-only"
                        type="file"
                        accept="image/png,image/jpeg,image/webp,image/gif"
                        node_ref=file_input
                        on:change=on_file_change
                    />
                    <Show
                        when=move || preview.get().is_some()
                        fallback=move || {
                            view! {
                                <span class="grid gap-1 text-center text-ink-2">
                                    <span class="mx-auto inline-flex text-ink-3">
                                        <Ico icon=LuCloudUpload size=26/>
                                    </span>
                                    <strong>
                                        {move || {
                                            if dragging.get() { "Drop it" } else { "Drop a file, or choose one" }
                                        }}
                                    </strong>
                                    // Kept: the format/size line prevents a
                                    // failed upload, and these values mirror
                                    // MAX_UPLOAD_BYTES and the accept list
                                    // rather than being restated by hand.
                                    <small>"PNG, JPG, WEBP, GIF · 12 MB max"</small>
                                </span>
                            }
                        }
                    >
                        <img class="max-h-[380px] max-w-full rounded" src=move || preview.get().unwrap_or_default()/>
                    </Show>
                </label>

                <Show when=move || filename.get().is_some()>
                    <p class="m-0 text-[0.85rem] text-ink-3">{move || filename.get().unwrap_or_default()}</p>
                </Show>

                <input
                    class="field"
                    type="text"
                    placeholder="Name this son"
                    maxlength="80"
                    node_ref=title_input
                />

                <button class="btn" type="submit" disabled=move || busy.get()>
                    {move || if busy.get() { "Working…" } else { "Upload" }}
                </button>
            </form>

            // The live step list. Every step is drawn from the start so the list
            // does not jump as rows appear; each is ticked once the server has
            // moved past it.
            <Show when=move || progress.get().is_some()>
                <ul
                    class="mt-4 grid gap-2 rounded-lg border border-line bg-surface p-4"
                    aria-live="polite"
                >
                    <For each=|| Step::ALL key=|s| *s let:step>
                        {
                            let current = move || match progress.get() {
                                Some(Progress::Running { step: s }) => Some(s),
                                _ => None,
                            };
                            let state = move || match current() {
                                Some(s) if s == step => "current",
                                Some(s) => {
                                    let at = Step::ALL.iter().position(|x| *x == s).unwrap_or(0);
                                    let mine = Step::ALL.iter().position(|x| *x == step).unwrap_or(0);
                                    if mine < at { "done" } else { "pending" }
                                }
                                None => "pending",
                            };
                            view! {
                                <li class=move || {
                                    match state() {
                                        "done" => "flex items-center gap-2.5 text-[0.9rem] text-ink-2",
                                        "current" => "flex items-center gap-2.5 text-[0.9rem] font-semibold text-ink",
                                        _ => "flex items-center gap-2.5 text-[0.9rem] text-ink-3",
                                    }
                                }>
                                    <span class="inline-flex w-4 justify-center">
                                        {move || match state() {
                                            "done" => view! { <span class="text-ok"><Ico icon=LuCheck size=14/></span> }.into_any(),
                                            // A pulsing dot, not a spinner: it is
                                            // one element, needs no keyframes of
                                            // its own, and the reduced-motion
                                            // rule in the stylesheet already
                                            // neutralises it.
                                            "current" => view! { <span class="h-2 w-2 animate-pulse rounded-full bg-accent"/> }.into_any(),
                                            _ => view! { <span class="h-1.5 w-1.5 rounded-full bg-line-strong"/> }.into_any(),
                                        }}
                                    </span>
                                    <span>{step.label()}</span>
                                </li>
                            }
                        }
                    </For>
                </ul>
            </Show>

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
                        UploadResult::Ok { son } if !son.is_public => {
                            view! {
                                <div class="mt-4 flex flex-col items-center gap-3 rounded-lg border border-line bg-surface p-4 text-center">
                                    <span class="inline-flex h-9 w-9 items-center justify-center rounded-full bg-surface-raised text-accent">
                                        <Ico icon=LuCircleAlert size=18/>
                                    </span>
                                    <p class="m-0 text-[0.9375rem] font-semibold text-ink">"Saved, waiting on review"</p>
                                    <p class="m-0 text-[0.85rem] text-ink-2">
                                        "We couldn't check this one automatically, so it won't appear until someone looks at it."
                                    </p>
                                </div>
                            }
                                .into_any()
                        }
                        UploadResult::Ok { son } => {
                            view! {
                                <div class="mt-4 flex flex-col items-center gap-3 rounded-lg border border-line bg-surface p-4 text-center">
                                    <span class="inline-flex h-9 w-9 items-center justify-center rounded-full bg-surface-raised text-ok">
                                        <Ico icon=LuCheck size=18/>
                                    </span>
                                    <p class="m-0 text-[0.9375rem] font-semibold text-ink">"Uploaded"</p>
                                    <A href=format!("/son/{}", son.slug) attr:class="btn">"View son"</A>
                                </div>
                            }
                                .into_any()
                        }
                        UploadResult::Rejected { reason } => {
                            view! {
                                <div class="mt-4 flex flex-col items-center gap-3 rounded-lg border border-line bg-surface p-4 text-center">
                                    <span class="inline-flex h-9 w-9 items-center justify-center rounded-full bg-surface-raised text-danger">
                                        <Ico icon=LuCircleAlert size=18/>
                                    </span>
                                    <p class="m-0 text-[0.9375rem] font-semibold text-ink">{reason}</p>
                                </div>
                            }
                                .into_any()
                        }
                        UploadResult::Error { message } => {
                            view! {
                                <div class="mt-4 flex flex-col items-center gap-3 rounded-lg border border-line bg-surface p-4 text-center">
                                    <span class="inline-flex h-9 w-9 items-center justify-center rounded-full bg-surface-raised text-danger">
                                        <Ico icon=LuCircleAlert size=18/>
                                    </span>
                                    <p class="m-0 text-[0.9375rem] font-semibold text-ink">{message}</p>
                                </div>
                            }
                                .into_any()
                        }
                    })
            }}
        </section>
    }
}
