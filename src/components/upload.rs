use leptos::prelude::*;
use leptos_meta::{Link, Meta, Title};
use leptos_router::components::A;

use crate::components::icon::{Ico, LuCheck, LuCircleAlert, LuCloudUpload};
use crate::models::UploadResult;
use crate::seo::absolute;

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

    let file_input: NodeRef<leptos::html::Input> = NodeRef::new();
    let title_input: NodeRef<leptos::html::Input> = NodeRef::new();

    // Every setter above is written only from the hydrate-only handlers below,
    // so the server build sees them as unused. The form is inert until wasm
    // takes over, which is expected — not a missing code path.
    #[cfg(not(feature = "hydrate"))]
    let _ = (set_preview, set_filename, set_busy, set_result, title_input);

    // Local object-URL preview so the uploader sees the son before committing.
    let on_file_change = move |_| {
        #[cfg(feature = "hydrate")]
        {
            if let Some(input) = file_input.get() {
                if let Some(files) = input.files() {
                    if let Some(file) = files.get(0) {
                        // File is a Blob subclass in the DOM; web-sys models that
                        // as AsRef, so no cast is needed.
                        let blob: &web_sys::Blob = file.as_ref();
                        if let Ok(url) = web_sys::Url::create_object_url_with_blob(blob) {
                            set_preview.set(Some(url));
                        }
                        set_filename.set(Some(file.name()));
                        set_result.set(None);
                    }
                }
            }
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

            leptos::task::spawn_local(async move {
                let form = web_sys::FormData::new().expect("FormData unavailable");
                let _ = form.append_with_blob_and_filename("son", &file, &file.name());
                let _ = form.append_with_str("title", &title);

                let outcome = gloo_net::http::Request::post("/api/upload")
                    .body(form)
                    .expect("FormData is a valid body")
                    .send()
                    .await;

                let parsed =
                    match outcome {
                        Ok(resp) => resp.json::<UploadResult>().await.unwrap_or_else(|e| {
                            UploadResult::Error {
                                message: format!("unexpected reply from the server: {e}"),
                            }
                        }),
                        Err(e) => UploadResult::Error {
                            message: format!("could not reach the server: {e}"),
                        },
                    };

                set_busy.set(false);
                set_result.set(Some(parsed));
            });
        }
    };

    view! {
        <Title text="Contribute — son collection"/>
        <Meta
            name="description"
            content="Contribute a son to the collection."
        />
        <Link rel="canonical" href=absolute("/upload")/>

        <section class="upload">
            // Just the task name. The paragraph that used to sit here narrated
            // the service ("Free, no account. Everything is checked before it
            // goes live…") -- none of which a visitor needs in order to pick a
            // file, so it is gone rather than reworded.
            <h1 class="page-title">"Contribute"</h1>

            <form on:submit=submit class="upload-form">
                <label class="drop">
                    <input
                        type="file"
                        accept="image/png,image/jpeg,image/webp,image/gif"
                        node_ref=file_input
                        on:change=on_file_change
                    />
                    <Show
                        when=move || preview.get().is_some()
                        fallback=|| {
                            view! {
                                <span class="drop-hint">
                                    <span class="drop-icon">
                                        <Ico icon=LuCloudUpload size=26/>
                                    </span>
                                    <strong>"Choose a file"</strong>
                                    // Kept: the format/size line prevents a
                                    // failed upload, and these values mirror
                                    // MAX_UPLOAD_BYTES and the accept list
                                    // rather than being restated by hand.
                                    <small>"PNG, JPG, WEBP, GIF · 12 MB max"</small>
                                </span>
                            }
                        }
                    >
                        <img class="drop-preview" src=move || preview.get().unwrap_or_default()/>
                    </Show>
                </label>

                <Show when=move || filename.get().is_some()>
                    <p class="filename">{move || filename.get().unwrap_or_default()}</p>
                </Show>

                <input
                    class="title-input"
                    type="text"
                    placeholder="Name this son"
                    maxlength="80"
                    node_ref=title_input
                />

                <button class="btn" type="submit" disabled=move || busy.get()>
                    {move || if busy.get() { "Uploading…" } else { "Upload" }}
                </button>
            </form>

            {move || {
                result
                    .get()
                    .map(|r| match r {
                        UploadResult::Ok { son } => {
                            view! {
                                <div class="outcome ok">
                                    <span class="outcome-icon">
                                        <Ico icon=LuCheck size=18/>
                                    </span>
                                    <p class="outcome-msg">"Uploaded"</p>
                                    <A href=format!("/son/{}", son.id) attr:class="btn">"View son"</A>
                                </div>
                            }
                                .into_any()
                        }
                        UploadResult::Rejected { reason, son_score, nsfw_score } => {
                            view! {
                                <div class="outcome rejected">
                                    <span class="outcome-icon">
                                        <Ico icon=LuCircleAlert size=18/>
                                    </span>
                                    <p class="outcome-msg">{reason}</p>
                                    <p class="scores">
                                        {format!(
                                            "sonness {:.0}% · nsfw {:.0}%",
                                            son_score * 100.0,
                                            nsfw_score * 100.0,
                                        )}
                                    </p>
                                </div>
                            }
                                .into_any()
                        }
                        UploadResult::Error { message } => {
                            view! {
                                <div class="outcome error">
                                    <span class="outcome-icon">
                                        <Ico icon=LuCircleAlert size=18/>
                                    </span>
                                    <p class="outcome-msg">{message}</p>
                                </div>
                            }
                                .into_any()
                        }
                    })
            }}
        </section>
    }
}
