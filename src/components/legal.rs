use leptos::prelude::*;
use leptos_meta::{Link, Meta, Title};

use crate::seo::absolute;

/// `/privacy`. Deliberately plain-language and specific about the actual
/// subprocessors this site uses (D1, R2, Cloudflare Tunnel, Google OAuth) --
/// a generic boilerplate policy would be actively misleading about what
/// really happens to a visitor's data here.
///
/// This is a good-faith draft covering what the site actually does, not a
/// substitute for real legal review before this goes fully public with a
/// wide audience.
#[component]
pub fn Privacy() -> impl IntoView {
    view! {
        <Title text="privacy policy — son collection"/>
        <Meta
            name="description"
            content="What son collection collects, why, and who it's shared with."
        />
        <Link rel="canonical" href=absolute("/privacy")/>

        <article class="max-w-[70ch] pt-5 [&_h2]:mb-2 [&_h2]:mt-7 [&_h2]:text-[1.2rem] [&_h2]:font-semibold [&_li]:my-1 [&_li]:leading-relaxed [&_li]:text-ink-2 [&_p]:leading-relaxed [&_p]:text-ink-2 [&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-5">
            <h1 class="m-0 mb-1.5 text-[2rem] font-bold tracking-tight">"Privacy"</h1>
            <p class="m-0 mb-6 text-[0.85rem] italic text-ink-3">"Last updated: this is a draft, not yet dated for a public launch."</p>

            <p>
                "This is a plain-language description of what son collection actually does with data. \
                It is a good-faith draft, not a substitute for real legal review before this site is \
                promoted widely."
            </p>

            <h2>"What we collect"</h2>
            <ul>
                <li>
                    "Images you upload, and the title/tags you give them. Uploads are public by \
                    default and stay that way unless removed (see \"Removal\" below)."
                </li>
                <li>
                    "An anonymous voter ID, stored in a cookie, used only to remember which sons \
                    you've liked and reported so you can't do either twice. It isn't linked to your \
                    name or email unless you sign in."
                </li>
                <li>
                    "If you sign in with Google: your name, email, and avatar, used to attribute \
                    uploads to you and to run the leaderboard. We don't request or store anything \
                    beyond basic profile info."
                </li>
                <li>"Standard server logs (IP address, timestamps, request paths) for security and abuse response."</li>
            </ul>

            <h2>"What we don't do"</h2>
            <ul>
                <li>"We don't sell data, to anyone, ever."</li>
                <li>"We don't run ads or ad-tech trackers."</li>
                <li>"We don't require an account to upload or browse."</li>
            </ul>

            <h2>"Who else sees it"</h2>
            <p>"Infrastructure providers that store or move data on our behalf, none of whom we permit to use it for their own purposes:"</p>
            <ul>
                <li>"Cloudflare -- database (D1), image storage (R2), and network routing (Tunnel)."</li>
                <li>"Google -- only if you choose to sign in, for authentication."</li>
            </ul>

            <h2>"Content moderation"</h2>
            <p>
                "Uploads are published immediately. Nothing analyses what an image contains before \
                it goes live -- there is no automated screening at present. What we do check is \
                whether the image is byte-for-byte identical to one already here, using a hash of \
                its pixels, which tells us nothing about the content itself."
            </p>
            <p>
                "Moderation is therefore after the fact and depends on reports. If you see something \
                that shouldn't be here, use the report button -- that is the mechanism, not a \
                fallback for one."
            </p>

            <h2>"Removal"</h2>
            <p>
                "Report any son you believe shouldn't be here using the report button on its page. \
                Reports are reviewed and repeatedly-flagged content is hidden pending review. If \
                you uploaded something and want it taken down, report it yourself with a note, or \
                reach out through the contact route below."
            </p>

            <h2>"Children's privacy"</h2>
            <p>"This site is not directed at children under 13, and we don't knowingly collect data from them."</p>

            <h2>"Changes"</h2>
            <p>"If this policy changes in a way that matters, the date at the top of this page will change too."</p>
        </article>
    }
}

/// `/tos`.
#[component]
pub fn Terms() -> impl IntoView {
    view! {
        <Title text="terms of service — son collection"/>
        <Meta name="description" content="The rules for using and contributing to son collection."/>
        <Link rel="canonical" href=absolute("/tos")/>

        <article class="max-w-[70ch] pt-5 [&_h2]:mb-2 [&_h2]:mt-7 [&_h2]:text-[1.2rem] [&_h2]:font-semibold [&_li]:my-1 [&_li]:leading-relaxed [&_li]:text-ink-2 [&_p]:leading-relaxed [&_p]:text-ink-2 [&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-5">
            <h1 class="m-0 mb-1.5 text-[2rem] font-bold tracking-tight">"Terms"</h1>
            <p class="m-0 mb-6 text-[0.85rem] italic text-ink-3">"Last updated: this is a draft, not yet dated for a public launch."</p>

            <p>"By using son collection, you agree to these terms. They're written in plain language on purpose."</p>

            <h2>"What you can upload"</h2>
            <ul>
                <li>"Only images you have the right to share. Don't upload someone else's copyrighted work as your own."</li>
                <li>"Nothing illegal, nothing that depicts real minors in a sexual context, no genuine explicit content."</li>
                <li>"No spam, and no images that don't have a good-faith \"son\" connection -- that's what the reporting reasons are for."</li>
            </ul>

            <h2>"The license you grant"</h2>
            <p>
                "By uploading, you grant son collection a non-exclusive, worldwide, royalty-free \
                license to host, display, and distribute the image as part of the site and its public \
                API/embeds (oEmbed, Open Graph previews, downloads). You keep whatever rights you had \
                in it -- this isn't a transfer of ownership, just permission to run the site."
            </p>

            <h2>"Moderation"</h2>
            <p>
                "Uploads are published without prior review and can be reported by anyone. We may \
                remove or hide any content, at any time, for any reason. Because nothing screens an \
                upload before it appears, content that breaks these rules can be visible until \
                someone reports it."
            </p>

            <h2>"No warranty"</h2>
            <p>
                "The site is provided \"as is,\" with no warranty of any kind. There is no automated \
                moderation, so nothing prevents an upload from appearing before a human has seen it."
            </p>

            <h2>"Limitation of liability"</h2>
            <p>
                "To the fullest extent the law allows, son collection and its operator aren't liable \
                for damages arising from your use of the site or content on it."
            </p>

            <h2>"Changes"</h2>
            <p>"These terms may change as the site does. Continued use after a change means you accept the new terms."</p>

            <h2>"Contact"</h2>
            <p>"For takedown requests or legal notices, report the specific son in question -- that routes straight to moderation."</p>
        </article>
    }
}
