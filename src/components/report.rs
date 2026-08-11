use leptos::prelude::*;

use crate::components::icon::{Ico, LuFlag};

use crate::api::report_son;
use crate::models::ReportReason;

/// The "this is not a son" flow: closed by default, opens into a reason
/// picker + optional message on click, rather than a single button that
/// reports with no detail — the detail is the whole point of Phase 3.
#[component]
pub fn ReportForm(son_id: String) -> impl IntoView {
    let (open, set_open) = signal(false);
    let (reason, set_reason) = signal(ReportReason::NotSon);
    let message_input: NodeRef<leptos::html::Textarea> = NodeRef::new();

    let report = Action::new(move |(reason, message): &(ReportReason, String)| {
        let id = son_id.clone();
        let reason = reason.as_str().to_string();
        let message = message.clone();
        async move { report_son(id, reason, Some(message)).await }
    });
    let reported = report.value();

    view! {
        <Show
            when=move || reported.get().is_none()
            fallback=|| view! { <p class="text-[0.9rem] text-ok">"Flagged. Someone will look."</p> }
        >
            <Show
                when=move || open.get()
                fallback=move || {
                    view! {
                        <button
                            class="icon-btn"
                            on:click=move |_| set_open.set(true)
                            aria-label="Report"
                            title="Report"
                        >
                            <Ico icon=LuFlag size=17/>
                        </button>
                    }
                }
            >
                <div class="mt-2.5 grid gap-2.5 rounded border border-line bg-surface p-3.5">
                    <fieldset class="m-0 grid gap-1.5 border-0 p-0">
                        <legend>"what's wrong with it?"</legend>
                        {ReportReason::all()
                            .into_iter()
                            .map(|r| {
                                view! {
                                    <label class="flex items-center gap-2 text-[0.9rem]">
                                        <input
                                            type="radio"
                                            name="report-reason"
                                            checked=move || reason.get() == r
                                            on:change=move |_| set_reason.set(r)
                                        />
                                        {r.label()}
                                    </label>
                                }
                            })
                            .collect_view()}
                    </fieldset>
                    <textarea
                        node_ref=message_input
                        class="field min-h-[60px] resize-y"
                        placeholder="anything else? (optional)"
                        maxlength="500"
                    ></textarea>
                    <div class="flex items-center gap-2.5">
                        <button
                            class="btn-quiet"
                            disabled=move || report.pending().get()
                            on:click=move |_| {
                                let message = message_input.get().map(|t| t.value()).unwrap_or_default();
                                report.dispatch((reason.get(), message));
                            }
                        >
                            {move || if report.pending().get() { "flagging…" } else { "submit report" }}
                        </button>
                        <button class="cursor-pointer border-0 bg-transparent p-0 text-inherit hover:text-danger" on:click=move |_| set_open.set(false)>
                            "Cancel"
                        </button>
                    </div>
                </div>
            </Show>
        </Show>
    }
}
