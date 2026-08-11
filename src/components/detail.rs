use leptos::prelude::*;
use leptos_meta::{Link, Meta, Title};
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;

use crate::api::get_son;
use crate::components::icon::{Ico, LuDownload, LuUserRound};
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

    view! {
        <Suspense fallback=|| view! { <p class="py-14 text-center text-ink-2">"finding the son…"</p> }>
            {move || {
                son.get()
                    .map(|res| match res {
                        Err(e) => {
                            view! { <p class="text-danger">{e.to_string()}</p> }.into_any()
                        }
                        Ok(None) => {
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

                                <article class="grid grid-cols-1 gap-4 pb-6 min-[860px]:grid-cols-[minmax(0,1fr)_300px] min-[860px]:items-start min-[860px]:gap-6">
                                    // Constrained figure, not a full-bleed
                                    // image: capped at 68vh so the son is
                                    // shown whole without pushing everything
                                    // else off-screen, and `contain` so a tall
                                    // son is letterboxed rather than cropped.
                                    //
                                    // The frame also hugs the image rather than
                                    // stretching to fill its grid column. Left
                                    // to stretch, a 640px-wide son sat in a
                                    // 932px panel with 150px of dead surface on
                                    // either side. All three caps are needed:
                                    // the column (100%), the son's own pixel
                                    // width (never upscale), and the width the
                                    // 68vh height cap implies for this aspect
                                    // (which is what a portrait son is actually
                                    // limited by). `min()` takes the tightest.
                                    <figure
                                        class="m-0 mx-auto flex max-h-[68vh] items-center justify-center overflow-hidden rounded-lg border border-line bg-surface p-3"
                                        style=format!(
                                            "max-width: min(100%, calc({}px + 1.5rem), calc(68vh * {:.4} + 1.5rem))",
                                            s.width,
                                            f64::from(s.width.max(1)) / f64::from(s.height.max(1)),
                                        )
                                    >
                                        <img
                                            class="h-auto max-h-[calc(68vh-1.5rem)] w-auto max-w-full rounded object-contain"
                                            src=s.orig_url.clone()
                                            alt=s.title.clone()
                                            width=s.width
                                            height=s.height
                                        />
                                    </figure>

                                    <div>
                                        <h1 class="m-0 mb-3 text-[1.375rem] font-bold tracking-tight lg:text-[1.75rem]">{s.title.clone()}</h1>

                                        <div class="flex flex-wrap items-center gap-2 pb-3 text-[0.8125rem] text-ink-3">
                                            <Ico icon=LuUserRound size=14/>
                                            <span>
                                                {match &s.uploader {
                                                    Some(u) => u.display_name.clone(),
                                                    None => "anonymous".to_string(),
                                                }}
                                            </span>
                                            <span class="text-line-strong">"·"</span>
                                            <span>
                                                {s.created_at.chars().take(10).collect::<String>()}
                                            </span>
                                        </div>


                                        // Icon-only actions. The count beside
                                        // the heart is data, not a label; every
                                        // control carries an aria-label and a
                                        // title so the meaning is available to
                                        // screen readers and on hover.
                                        // `border-t` only, not `border-y`: the
                                        // "More sons" section below draws its
                                        // own top rule, so a bottom rule here
                                        // left two hairlines 62px apart with
                                        // nothing between them.
                                        <div class="my-3 flex items-center gap-2 border-t border-line pt-3">
                                            <LikeButton
                                                id=s.id.clone()
                                                initial_count=s.likes
                                                initial_liked=s.liked_by_me
                                            />
                                            <a
                                                class="icon-btn"
                                                href=format!("/son/{}/download", s.id)
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
                                            <ReportForm son_id=s.id.clone()/>
                                        </div>
                                    </div>
                                </article>

                                <MoreSons exclude=s.id.clone()/>
                            }
                                .into_any()
                        }
                    })
            }}
        </Suspense>
    }
}
