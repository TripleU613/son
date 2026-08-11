use leptos::prelude::*;
use leptos_meta::{provide_meta_context, MetaTags, Stylesheet, Title};
use leptos_router::components::{Route, Router, Routes, A};
use leptos_router::{path, SsrMode};

use crate::components::detail::SonDetail;
use crate::components::gallery::Gallery;
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
    view! {
        <header class="nav">
            <A href="/" attr:class="brand">
                <span class="brand-word">"son"</span>
                <span class="brand-tears">"😭😭😭😭😭"</span>
            </A>
            <nav class="nav-links">
                <A href="/">"gallery"</A>
                <A href="/upload">"contribute a son"</A>
            </nav>
        </header>
    }
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
