use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::components::{Route, Router, Routes, A};
use leptos_router::hooks::use_location;
use leptos_router::{path, SsrMode};

use crate::api::current_user;
use crate::components::admin::Admin;
use crate::components::detail::SonDetail;
use crate::components::gallery::Gallery;
use crate::components::leaderboard::Leaderboard;
use crate::components::upload::Upload;

/// The HTML document. Server-side only entry point; `HydrationScripts` injects
/// the wasm loader so the client picks up where SSR left off.
pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="utf-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1"/>
                <AutoReload options=options.clone()/>
                <HydrationScripts options/>
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
        <Stylesheet id="leptos" href="/pkg/soncollection.css"/>
        <Title text="son collection"/>

        <Router>
            <Nav/>
            <main class="wrap">
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
                    // Not SsrMode::Async: it carries no SEO-relevant content
                    // and is gated server-side in every fn it calls anyway, so
                    // there is nothing here for out-of-order streaming to leak.
                    <Route path=path!("/admin") view=Admin/>
                </Routes>
            </main>
            <footer class="foot">
                <span>"every image is somebody's son"</span>
            </footer>
        </Router>
    }
}

#[component]
fn Nav() -> impl IntoView {
    let location = use_location();
    let user = Resource::new(|| (), |_| current_user());

    view! {
        <header class="nav">
            <A href="/" attr:class="brand">
                <span class="brand-word">"son"</span>
                <span class="brand-tears">"😭😭😭😭😭"</span>
            </A>
            <nav class="nav-links">
                <A href="/">"gallery"</A>
                <A href="/upload">"contribute a son"</A>
                <A href="/leaderboard">"leaderboard"</A>
                <Suspense fallback=|| ()>
                    {move || {
                        user.get()
                            .map(|res| match res {
                                Ok(Some(u)) => {
                                    view! {
                                        <span class="nav-user">
                                            {u.is_admin.then(|| view! { <A href="/admin">"admin"</A> })}
                                            {u.avatar_url.clone().map(|src| {
                                                view! { <img class="nav-avatar" src=src alt=""/> }
                                            })}
                                            <span>{u.display_name.clone()}</span>
                                            <form method="post" action="/auth/logout" class="nav-logout">
                                                <button type="submit" class="link-btn">"sign out"</button>
                                            </form>
                                        </span>
                                    }
                                        .into_any()
                                }
                                _ => {
                                    // Ok(None) (never signed in, or Google
                                    // sign-in isn't configured yet) and Err
                                    // (couldn't reach D1) get the same
                                    // sign-in link — nothing useful to tell
                                    // a visitor apart between those here.
                                    let return_to = location.pathname.get();
                                    let href = format!(
                                        "/auth/google/login?return_to={}",
                                        urlencode(&return_to),
                                    );
                                    view! { <a href=href>"sign in"</a> }.into_any()
                                }
                            })
                    }}
                </Suspense>
            </nav>
        </header>
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
