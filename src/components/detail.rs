use leptos::prelude::*;
use leptos_meta::{Link, Meta, Title};
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;

use crate::api::get_son;
use crate::components::icon::{Ico, LuDownload, LuSun, LuUserRound};
use crate::components::like::LikeButton;
use crate::components::report::ReportForm;
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
    format!(
        "{} — {} in the son collection.{}",
        s.title,
        s.sonness_label(),
        by
    )
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
        <Suspense fallback=|| view! { <p class="loading">"finding the son…"</p> }>
            {move || {
                son.get()
                    .map(|res| match res {
                        Err(e) => {
                            view! { <p class="error">{e.to_string()}</p> }.into_any()
                        }
                        Ok(None) => {
                            view! {
                                <section class="empty">
                                    <h1>"No such son."</h1>
                                    <A href="/">"back to the collection"</A>
                                </section>
                            }
                                .into_any()
                        }
                        Ok(Some(s)) => {
                            // A plain one-time conditional, not <Show>: `when`
                            // there expects a reactive closure for a condition
                            // that can change after the initial render, and
                            // tags don't change once the son has loaded --
                            // using it anyway is what produced a real
                            // Fn-vs-FnOnce compile error from the nested
                            // closure capturing `tags` by move.
                            let tag_chips = (!s.tags.is_empty()).then(|| {
                                let tags = s.tags.clone();
                                view! {
                                    <div class="detail-tags">
                                        <For each=move || tags.clone() key=|t| t.slug.clone() let:tag>
                                            <A href=format!("/tag/{}", tag.slug) attr:class="tag-chip">
                                                {tag.name.clone()}
                                            </A>
                                        </For>
                                    </div>
                                }
                            });
                            let description = describe(&s);
                            let page_url = absolute(&format!("/son/{}", s.id));
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

                                <article class="detail">
                                    <img
                                        class="detail-img"
                                        src=s.orig_url.clone()
                                        alt=s.title.clone()
                                        width=s.width
                                        height=s.height
                                    />
                                    <div class="detail-meta">
                                        <h1 class="detail-title">{s.title.clone()}</h1>

                                        // Sun level: icon + value, no label
                                        // column. This is the site's own
                                        // metric, so it leads the metadata.
                                        <div class="detail-stat">
                                            <Ico icon=LuSun size=16/>
                                            <span>{s.sonness_label()}</span>
                                        </div>

                                        // Contributor and date on one thin
                                        // line, replacing a four-row <dl> of
                                        // label/value pairs.
                                        <div class="detail-byline">
                                            <Ico icon=LuUserRound size=15/>
                                            <span>
                                                {match &s.uploader {
                                                    Some(u) => u.display_name.clone(),
                                                    None => "anonymous".to_string(),
                                                }}
                                            </span>
                                            <span class="detail-sep">"·"</span>
                                            <span>{s.created_at.chars().take(10).collect::<String>()}</span>
                                            <span class="detail-sep">"·"</span>
                                            <span>{format!("{}\u{00D7}{}", s.width, s.height)}</span>
                                        </div>

                                        {tag_chips}

                                        // Engagement row: icon + value, one
                                        // line, thin separators instead of
                                        // nesting each action in its own card.
                                        <div class="detail-actions">
                                            <LikeButton
                                                id=s.id.clone()
                                                initial_count=s.likes
                                                initial_liked=s.liked_by_me
                                            />
                                            <a
                                                class="action-btn"
                                                href=format!("/son/{}/download", s.id)
                                                aria-label="Download this son"
                                            >
                                                <Ico icon=LuDownload size=15/>
                                                <span>"Download"</span>
                                            </a>
                                        </div>

                                        <ReportForm son_id=s.id.clone()/>
                                    </div>
                                </article>
                            }
                                .into_any()
                        }
                    })
            }}
        </Suspense>
    }
}
