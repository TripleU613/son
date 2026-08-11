use leptos::prelude::*;
use leptos_meta::{Link, Meta, Title};
use leptos_router::components::A;
use leptos_router::hooks::use_params_map;

use crate::api::get_son;
use crate::components::like::LikeButton;
use crate::components::report::ReportForm;

/// Link unfurlers (Discord, Twitter, Slack) reject relative `og:image` URLs, so
/// these must be absolute. Set `SITE_ORIGIN` (e.g. `https://soncollection.com`);
/// without it the tag stays relative and previews will have no image.
fn absolute(url: &str) -> String {
    #[cfg(feature = "ssr")]
    {
        if let Ok(origin) = std::env::var("SITE_ORIGIN") {
            let origin = origin.trim_end_matches('/');
            if !origin.is_empty() {
                return format!("{origin}{url}");
            }
        }
    }
    url.to_string()
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
                            view! {
                                <Title text=format!("{} — son collection", s.title)/>
                                <Meta property="og:title" content=s.title.clone()/>
                                <Meta property="og:image" content=absolute(&s.orig_url)/>
                                <Meta property="og:type" content="image"/>
                                <Meta name="twitter:card" content="summary_large_image"/>
                                // oEmbed discovery: lets embedders that check
                                // <link rel="alternate"> (WordPress, many wikis)
                                // find this without knowing the endpoint shape
                                // up front, the way Discord/Twitter's OG parsing
                                // already does implicitly.
                                <Link
                                    rel="alternate"
                                    type_="application/json+oembed"
                                    title=s.title.clone()
                                    href=format!(
                                        "{}?url={}",
                                        absolute("/oembed"),
                                        absolute(&format!("/son/{}", s.id)),
                                    )
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
                                        <h1>{s.title.clone()}</h1>
                                        <div class="detail-like">
                                            <LikeButton
                                                id=s.id.clone()
                                                initial_count=s.likes
                                                initial_liked=s.liked_by_me
                                            />
                                        </div>
                                        {tag_chips}
                                        <dl>
                                            <dt>"sonness"</dt>
                                            <dd>{s.sonness_label()}</dd>
                                            <dt>"dimensions"</dt>
                                            <dd>{format!("{}×{}", s.width, s.height)}</dd>
                                            <dt>"collected"</dt>
                                            <dd>{s.created_at.clone()}</dd>
                                            <dt>"contributed by"</dt>
                                            <dd>
                                                {match &s.uploader {
                                                    Some(u) => u.display_name.clone(),
                                                    None => "anonymous".to_string(),
                                                }}
                                            </dd>
                                        </dl>

                                        <a
                                            class="download-btn"
                                            href=format!("/son/{}/download", s.id)
                                        >
                                            "download"
                                        </a>

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
