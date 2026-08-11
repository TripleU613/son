use leptos::prelude::*;
use leptos_meta::{provide_meta_context, HashedStylesheet, MetaTags, Title};
use leptos_router::components::{Route, Router, Routes, A};
use leptos_router::hooks::use_location;
use leptos_router::{path, SsrMode};

use crate::api::current_user;
use crate::components::admin::Admin;
use crate::components::detail::SonDetail;
use crate::components::gallery::Gallery;
use crate::components::icon::{Ico, LuCirclePlus, LuImage, LuLogOut, LuTrophy, LuUserRound};
use crate::components::leaderboard::Leaderboard;
use crate::components::legal::{Privacy, Terms};
use crate::components::search::SearchPage;
use crate::components::search_box::SearchBox;
use crate::components::tag_page::TagPage;
use crate::components::upload::Upload;

/// An inline SVG favicon (the crying-face emoji on the accent yellow) --
/// no image asset to generate or keep in sync with the brand color, and it
/// renders crisp at every size browsers ask for.
const FAVICON: &str = "data:image/svg+xml,\
<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'>\
<text y='.9em' font-size='90'>%F0%9F%98%AD</text>\
</svg>";

/// The HTML document. Server-side only entry point; `HydrationScripts` injects
/// the wasm loader so the client picks up where SSR left off.
pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <link rel="icon" href=FAVICON/>
                <AutoReload options=options.clone()/>
                <HydrationScripts options=options.clone()/>
                // Emitted here rather than as a <Stylesheet> inside App: the
                // hashed filename lives in a file next to the binary, so this
                // needs LeptosOptions, which only exists server-side. See
                // `hash-files` in Cargo.toml for why the hash matters.
                <HashedStylesheet options=options.clone() id="leptos"/>
                <MetaTags/>
            </head>
            <body>
                <App/>
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Title text="son collection"/>

        <Router>
            // One shell, rearranged entirely by CSS. The primary nav exists
            // exactly once in the DOM and becomes either the desktop sidebar's
            // link list or the mobile bottom bar -- rendering two copies would
            // duplicate landmarks, and branching on window.innerWidth in Rust
            // would invite the hydration mismatch that already took down the
            // wasm module once this session.
            <div class="app">
                <TopBar/>
                <Rail/>
                <main class="content">
                    <Routes fallback=NotFound>
                    // Async mode on the two data-driven routes: the default
                    // out-of-order streaming flushes <head> before resources
                    // resolve, so og:/twitter: tags and card markup ended up in
                    // a JS-swapped <template> where unfurlers never see them.
                    <Route path=path!("/") view=Gallery ssr=SsrMode::Async/>
                    <Route path=path!("/son/:id") view=SonDetail ssr=SsrMode::Async/>
                    // No server data; streaming is fine here.
                    <Route path=path!("/upload") view=Upload/>
                    <Route path=path!("/leaderboard") view=Leaderboard ssr=SsrMode::Async/>
                    <Route path=path!("/tag/:slug") view=TagPage ssr=SsrMode::Async/>
                    <Route path=path!("/search") view=SearchPage ssr=SsrMode::Async/>
                    // Not SsrMode::Async: it carries no SEO-relevant content
                    // and is gated server-side in every fn it calls anyway, so
                    // there is nothing here for out-of-order streaming to leak.
                    <Route path=path!("/admin") view=Admin/>
                        <Route path=path!("/privacy") view=Privacy/>
                        <Route path=path!("/tos") view=Terms/>
                    </Routes>
                </main>
            </div>
        </Router>
    }
}

/// The mobile-only top bar: brand on the left, search and account on the
/// right. Hidden by CSS at the desktop breakpoint, where the brand lives in the
/// sidebar and search moves into the gallery's own utility row. Because it is
/// `display: none` there, only one brand link is ever in the accessibility
/// tree despite appearing twice in the markup.
#[component]
fn TopBar() -> impl IntoView {
    view! {
        <header class="topbar">
            <A href="/" attr:class="brand" attr:aria-label="son collection, home">
                <span class="brand-word">"son"</span>
                <span class="brand-tears" aria-hidden="true">
                    <span>"\u{1F62D}"</span>
                    <span>"\u{1F62D}"</span>
                    <span>"\u{1F62D}"</span>
                </span>
            </A>
            <div class="topbar-actions">
                <SearchBox extra_class="searchbox--bar"/>
                <AccountAction compact=true/>
            </div>
        </header>
    }
}

/// The primary navigation. A left sidebar on desktop; a fixed bottom bar on
/// mobile. Same three links either way, in one place in the DOM.
#[component]
fn Rail() -> impl IntoView {
    view! {
        <aside class="rail">
            <A href="/" attr:class="rail-brand" attr:aria-label="son collection, home">
                <span class="brand-word">"son"</span>
                <span class="brand-tears" aria-hidden="true">
                    <span>"\u{1F62D}"</span>
                    <span>"\u{1F62D}"</span>
                    <span>"\u{1F62D}"</span>
                    <span>"\u{1F62D}"</span>
                    <span>"\u{1F62D}"</span>
                </span>
            </A>

            <nav class="rail-nav" aria-label="main">
                // exact, not the default prefix match: every route starts with
                // "/", so without this the Gallery link would render
                // aria-current="page" on every page in the app.
                <A href="/" attr:class="rail-link" exact=true>
                    <Ico icon=LuImage/>
                    <span class="rail-label">"Gallery"</span>
                </A>
                <A href="/upload" attr:class="rail-link">
                    <Ico icon=LuCirclePlus/>
                    <span class="rail-label">"Contribute"</span>
                </A>
                <A href="/leaderboard" attr:class="rail-link">
                    <Ico icon=LuTrophy/>
                    <span class="rail-label">"Leaderboard"</span>
                </A>
            </nav>

            // Desktop-only tail: account plus the legal links that used to sit
            // in a page-wide footer competing with the bottom nav on mobile.
            <div class="rail-foot">
                <AccountAction compact=false/>
                <div class="rail-legal">
                    <A href="/privacy">"Privacy"</A>
                    <A href="/tos">"Terms"</A>
                </div>
            </div>
        </aside>
    }
}

/// Sign-in link, or the signed-in user's avatar/name plus sign-out. `compact`
/// drops the name and legal tail for the mobile top bar, where there is only
/// room for the avatar.
#[component]
fn AccountAction(compact: bool) -> impl IntoView {
    let location = use_location();
    let user = Resource::new(|| (), |_| current_user());

    view! {
        <Suspense fallback=|| ()>
            {move || {
                user.get()
                    .map(|res| match res {
                        Ok(Some(u)) => {
                            view! {
                                <div class="account">
                                    {u.is_admin
                                        .then(|| {
                                            view! {
                                                <A href="/admin" attr:class="account-admin">"Admin"</A>
                                            }
                                        })}
                                    {u
                                        .avatar_url
                                        .clone()
                                        .map(|src| view! { <img class="account-avatar" src=src alt=""/> })}
                                    {(!compact)
                                        .then(|| {
                                            view! {
                                                <span class="account-name">{u.display_name.clone()}</span>
                                            }
                                        })}
                                    <form method="post" action="/auth/logout" class="account-out">
                                        <button
                                            type="submit"
                                            class="icon-btn"
                                            aria-label="Sign out"
                                            title="Sign out"
                                        >
                                            <Ico icon=LuLogOut size=16/>
                                        </button>
                                    </form>
                                </div>
                            }
                                .into_any()
                        }
                        _ => {
                            // Ok(None) (never signed in, or Google sign-in is
                            // not configured yet) and Err (couldn't reach D1)
                            // get the same link -- nothing useful to tell a
                            // visitor apart between those here.
                            let return_to = location.pathname.get();
                            let href = format!(
                                "/auth/google/login?return_to={}",
                                urlencode(&return_to),
                            );
                            if compact {
                                view! {
                                    <a class="icon-btn" href=href aria-label="Sign in" title="Sign in">
                                        <Ico icon=LuUserRound size=18/>
                                    </a>
                                }
                                    .into_any()
                            } else {
                                view! {
                                    <a class="rail-signin" href=href>
                                        <Ico icon=LuUserRound size=16/>
                                        <span>"Sign in"</span>
                                    </a>
                                }
                                    .into_any()
                            }
                        }
                    })
            }}
        </Suspense>
    }
}

/// Percent-encode a path for use as a query value. `return_to` is always a
/// same-origin path (checked again server-side in oauth_route::login), so
/// only the characters that would break a query string need escaping.
fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}

#[component]
fn NotFound() -> impl IntoView {
    view! {
        <Title text="no son here — son collection"/>
        <section class="empty">
            <h1>"404"</h1>
            <p>"No son at this address. Son 😭"</p>
            <A href="/">"back to the collection"</A>
        </section>
    }
}
