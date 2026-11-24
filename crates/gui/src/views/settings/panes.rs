//! The remaining Settings sections.
//!
//! Several of these describe capabilities the library does not have yet
//! (device selection, notification routing). They say so rather than offering
//! a control that quietly does nothing — a switch that does not switch is a
//! worse answer than an honest note about what is missing.

use gpui::{
    AnyElement, App, Entity, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
    div, prelude::FluentBuilder as _,
};
use gpui_component::ActiveTheme as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::Input;
use gpui_component::switch::Switch;
use gpui_component::{Disableable as _, Icon, IconName, Selectable as _, Sizable as _};

use oxidezap_core::{LogLevel, PrivacySettingCategory, PrivacySettingValue, UsernameLookupResult};

use crate::app::{SettingsSection, WhatsAppApp};
use crate::components::ProductIcon;
use crate::components::parts;
use crate::theme::Metrics;

pub fn render(
    section: SettingsSection,
    app: &WhatsAppApp,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> AnyElement {
    let body = match section {
        SettingsSection::Account => account(app, entity, metrics, cx),
        SettingsSection::Notifications => notifications(metrics, cx),
        SettingsSection::AudioVideo => super::devices::render(app, entity.clone(), metrics, cx),
        SettingsSection::Contacts => contacts(app, entity, metrics, cx),
        SettingsSection::Privacy => privacy(app, entity.clone(), metrics, cx),
        SettingsSection::Storage => storage(app, entity, metrics, cx),
        SettingsSection::Advanced => advanced(entity, metrics, cx),
        SettingsSection::Plugins => plugins(app, entity, metrics, cx),
        // Rendered by their own modules.
        SettingsSection::Appearance | SettingsSection::Shortcuts => div().into_any_element(),
    };

    div()
        .flex()
        .flex_col()
        .gap(metrics.space_xxl())
        .max_w(metrics.reading_width())
        .child(body)
        .into_any_element()
}

/// A titled block: a label, then its content.
///
/// The two elements are named type parameters rather than `impl IntoElement`
/// so that the return type can say `use<H, C>`. An argument-position `impl
/// Trait` introduces a parameter with no name, and a `use<..>` list has to
/// name every parameter it captures — so with them spelled that way the bound
/// cannot be written at all, and the helper silently inherits `metrics`'s
/// lifetime the way the rule exists to prevent.
pub fn group<H: IntoElement, C: IntoElement>(
    heading: H,
    content: C,
    metrics: Metrics,
) -> impl IntoElement + use<H, C> {
    div()
        .flex()
        .flex_col()
        // The heading belongs to what follows it, so it sits closer to its
        // content than the block does to the next block.
        .gap(metrics.space_lg())
        .child(heading)
        .child(content)
}

/// Every plugin this daemon knows about, and the two things that change the
/// list.
///
/// One list, in one shape. It used to be two — the plugins that published a
/// surface, drawn as cards, and below them the files that did not load, drawn
/// as a table of key/value rows with a column of identical Remove buttons
/// underneath it and nothing tying a button to a row. A module that failed to
/// load is still a plugin in the folder; the difference is a word in its
/// header, not a second layout.
fn plugins(
    app: &WhatsAppApp,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> AnyElement {
    let ctx = crate::components::PluginContext {
        entity: entity.clone(),
        metrics,
    };
    let home = crate::platform::plugins::home();

    // Everything in the folder that published nothing. A module that fails to
    // parse, answers the wrong ABI version or traps in `oxi_init` has no
    // surface at all, so a screen drawn from the surfaces alone leaves the
    // one file somebody most needs to remove with no control anywhere — and
    // it goes on spending the folder's budget at every load.
    let unloaded: Vec<String> = app
        .installed_plugins(cx)
        .unwrap_or_default()
        .iter()
        .filter(|id| !app.plugins(cx).iter().any(|surface| &surface.id == *id))
        .cloned()
        .collect();

    let rows = app.plugins(cx).len() + unloaded.len();
    let list = div()
        .flex()
        .flex_col()
        .gap(metrics.space_lg())
        .children(app.plugins(cx).iter().map(|surface| {
            crate::components::plugin_ui::settings_entry(
                surface,
                app,
                &ctx,
                removal(&surface.id, app, entity.clone(), metrics, cx),
                cx,
            )
            .into_any_element()
        }))
        .children(unloaded.iter().map(|id| {
            crate::components::plugin_ui::unloaded_entry(
                id,
                removal(id, app, entity.clone(), metrics, cx),
                metrics,
                cx,
            )
            .into_any_element()
        }));

    group(
        label(crate::tr!(Plugins), metrics, cx),
        div()
            .flex()
            .flex_col()
            .gap(metrics.space_lg())
            // Prose, not a key/value row: "None loaded" against a sentence
            // read as a setting whose value happened to be an instruction.
            .child(if rows == 0 {
                empty(home.nothing_loaded(), metrics, cx).into_any_element()
            } else {
                list.into_any_element()
            })
            .child(plugin_controls(entity, metrics, cx)),
        metrics,
    )
    .into_any_element()
}

/// The control that takes one plugin back out of the folder, where this front
/// end has a folder to take it out of.
///
/// Inside the plugin's own card, which is the whole of what makes it
/// unambiguous. `None` twice over: where the folder is another process's, and
/// where this plugin's file has already gone — a Remove button over a file
/// that is not there is one whose second press answers "not found".
///
/// Absent from a folder that has been *read*. A listing nobody has answered
/// yet is not the folder saying no — the read is a task, so the first frame
/// after Settings opens has none of it, and taking that for absence would
/// tell somebody every plugin they are running had been removed.
fn removal(
    id: &str,
    app: &WhatsAppApp,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> Option<gpui::AnyElement> {
    if app
        .installed_plugins(cx)
        .is_some_and(|ids| !ids.iter().any(|f| f == id))
    {
        // Said rather than left blank: a control that vanishes tells nobody
        // anything, which is the same reason a stopped plugin's widgets stay
        // on screen beside their reason.
        return Some(
            div()
                .flex_shrink_0()
                .text_size(metrics.text_meta())
                .text_color(cx.theme().muted_foreground)
                .child(crate::tr!(Removed))
                .into_any_element(),
        );
    }
    let id = id.to_owned();
    Some(
        Button::new(gpui::SharedString::from(format!("remove-plugin-{id}")))
            .label(crate::tr!(Remove))
            .ghost()
            .small()
            .cursor_pointer()
            .on_click(move |_, _window, cx| {
                let id = id.clone();
                entity.update(cx, |app, cx| app.remove_plugin(id, cx));
            })
            .flex_shrink_0()
            .into_any_element(),
    )
}

/// What stands where the list would be.
fn empty(what: &'static str, metrics: Metrics, cx: &App) -> impl IntoElement + use<> {
    div()
        .w_full()
        .rounded(metrics.radius_md())
        .border_1()
        .border_color(cx.theme().border)
        .px(metrics.space_lg())
        .py(metrics.space_xl())
        .text_size(metrics.text_small())
        .text_color(cx.theme().muted_foreground)
        .child(what)
}

/// The row under the list: reload, and add.
///
/// Both are drawn everywhere, because both are requests on the wire: the
/// folder belongs to whichever daemon is running the plugins, and adding to
/// it and re-reading it are things every front end asks that daemon for. A
/// desktop window's Reload is also for somebody who has just dropped a
/// `.wasm` beside `oxidezapd` by hand; a follower tab's is for somebody whose
/// host is another tab.
///
/// With a line saying what Reload does, because "reload" over a list of
/// running things is a word somebody is right to hesitate over.
fn plugin_controls(
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> impl IntoElement + use<> {
    let installer = entity.clone();
    div()
        .flex()
        .items_center()
        .gap(metrics.space_md())
        .px(metrics.space_xs())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(metrics.text_meta())
                .text_color(cx.theme().muted_foreground)
                .child(crate::tr!(
                    ReloadingStopsEveryPluginAndStartsWhatIsInTheFolderNow
                )),
        )
        .child(
            Button::new("reload-plugins")
                .label(crate::tr!(ReloadPlugins))
                .ghost()
                .cursor_pointer()
                .on_click(move |_, _window, cx| {
                    entity.update(cx, |app, cx| app.reload_plugins(cx));
                }),
        )
        .child(
            Button::new("install-plugin")
                .label(crate::tr!(AddAPlugin))
                .outline()
                .cursor_pointer()
                .on_click(move |_, _window, cx| {
                    installer.update(cx, |app, cx| app.install_plugin(cx));
                }),
        )
}

/// A short all-caps section label.
pub fn label(text: &'static str, metrics: Metrics, cx: &App) -> impl IntoElement + use<> {
    div()
        .font_family(cx.theme().mono_font_family.clone())
        .text_size(metrics.text_micro())
        .text_color(parts::subtle(cx))
        .child(text)
}

/// A block of label/value lines, drawn as one surface.
///
/// Bordered rather than a bare list. Three lines on the background with
/// hairlines under them read as the leftovers of a table someone deleted;
/// a settings pane is a set of panels, and this is one panel.
pub fn card(lines: Vec<(String, String)>, metrics: Metrics, cx: &App) -> impl IntoElement + use<> {
    let last = lines.len().saturating_sub(1);
    div()
        .w_full()
        .flex()
        .flex_col()
        .rounded(metrics.radius_md())
        .overflow_hidden()
        .bg(cx.theme().secondary)
        .border_1()
        .border_color(cx.theme().border)
        .children(
            lines
                .into_iter()
                .enumerate()
                .map(|(ix, (key, value))| row(key, value, ix == last, metrics, cx)),
        )
}

/// One label/value line.
fn row(
    key: String,
    value: String,
    is_last: bool,
    metrics: Metrics,
    cx: &App,
) -> impl IntoElement + use<> {
    div()
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .gap(metrics.space_xl())
        .px(metrics.space_lg())
        .py(metrics.space_lg())
        // The panel's own edge is the last line's rule.
        .when(!is_last, |el| {
            el.border_b_1().border_color(cx.theme().border)
        })
        .child(
            div()
                .flex_shrink_0()
                .text_size(metrics.text_secondary())
                .text_color(cx.theme().muted_foreground)
                .child(key),
        )
        .child(
            // The value is the side that gives: a long push name or a path
            // should end in an ellipsis rather than push the label off screen.
            parts::one_line()
                .flex_1()
                .min_w_0()
                .font_family(cx.theme().mono_font_family.clone())
                .text_size(metrics.text_meta())
                .text_color(cx.theme().foreground)
                .text_right()
                .child(value),
        )
}

/// What a section is waiting on, stated plainly.
fn pending(text: &'static str, metrics: Metrics, cx: &App) -> impl IntoElement + use<> {
    div()
        .w_full()
        .flex()
        // Top, not centre: these run to two or three lines, and an icon
        // floating in the middle of a paragraph reads as unattached to it.
        .items_start()
        .gap(metrics.space_lg())
        .p(metrics.space_lg())
        .rounded(metrics.radius_md())
        .bg(cx.theme().secondary)
        .border_1()
        .border_color(cx.theme().border)
        .child(
            Icon::new(IconName::Info)
                .size(metrics.icon_small())
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground),
        )
        .child(
            // Without a shrinkable flex child the sentence lays out to its
            // natural width and runs off the pane; this is what makes it wrap.
            div()
                .flex_1()
                .min_w_0()
                .text_size(metrics.text_small())
                .text_color(cx.theme().muted_foreground)
                .child(text),
        )
}

fn account(
    app: &WhatsAppApp,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> AnyElement {
    let account = app.account_summary();

    let mut lines = vec![(
        crate::tr!(Name).to_string(),
        account
            .as_ref()
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "—".to_string()),
    )];
    if let Some(jid) = app.account_jid() {
        lines.push((
            crate::tr!(Number).to_string(),
            // The user part alone: the server suffix is noise to a reader
            // checking which account this is.
            jid.split('@').next().unwrap_or(jid).to_string(),
        ));
    }
    lines.push((
        crate::tr!(Status).to_string(),
        account
            .as_ref()
            .map(|a| a.status.clone())
            .unwrap_or_else(|| crate::tr!(NotLinked).to_string()),
    ));

    let linked = div()
        .flex()
        .flex_col()
        .gap(metrics.space_lg())
        .child(card(lines, metrics, cx))
        .when(app.has_retained_history(), |this| {
            let open = entity.clone();
            this.child(
                Button::new("account-retained-history")
                    .label(crate::tr!(OpenRetainedHistory))
                    .on_click(move |_, window, cx| {
                        open.update(cx, |app, cx| app.open_retained_history(window, cx));
                    }),
            )
        });

    div()
        .flex()
        .flex_col()
        .gap(metrics.space_xxl())
        .child(
            group(
                label(crate::tr!(LinkedDevice), metrics, cx),
                linked,
                metrics,
            )
            .into_any_element(),
        )
        .child(
            group(
                label(crate::tr!(Accounts), metrics, cx),
                account_list(app, entity, metrics, cx),
                metrics,
            )
            .into_any_element(),
        )
        .into_any_element()
}

/// The account set the control plane has published.
///
/// A row carries only what `AccountOverview` actually has: the local id, the
/// runtime status, and whether this window chose it. Names and phone numbers
/// would have to be invented from an id, which is worse than showing less.
fn account_list(
    app: &WhatsAppApp,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> AnyElement {
    let has_control = app.has_account_control();
    let active = app.active_account();
    let accounts = app.accounts().unwrap_or_default();
    let mut list = div().flex().flex_col().gap(metrics.space_lg());

    if cfg!(target_family = "wasm") {
        // A page owns one daemon in its address space; the desktop window is
        // the only front end with a multi-account switcher in this slice.
        list = list.child(pending(crate::tr!(AccountSwitcherNativeOnly), metrics, cx));
    } else {
        list = list
            .children(accounts.iter().map(|overview| {
                account_row(
                    overview,
                    active,
                    app.account_request(overview.id),
                    has_control,
                    entity.clone(),
                    metrics,
                    cx,
                )
            }))
            .when(accounts.is_empty(), |this| {
                this.child(pending(crate::tr!(NoAccountsReported), metrics, cx))
            });

        let add = entity.clone();
        let refresh = entity.clone();
        list = list
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(metrics.space_md())
                    .child(
                        Button::new("accounts-add")
                            .label(crate::tr!(AddAccount))
                            .disabled(!has_control)
                            .on_click(move |_, window, cx| {
                                add.update(cx, |app, cx| app.add_account(window, cx));
                            }),
                    )
                    .child(
                        Button::new("accounts-refresh")
                            .label(crate::tr!(RefreshAccounts))
                            .disabled(!has_control)
                            .on_click(move |_, _, cx| {
                                refresh.update(cx, |app, cx| app.refresh_accounts(cx));
                            }),
                    ),
            )
            .when(!has_control, |this| {
                this.child(pending(crate::tr!(AccountControlUnavailable), metrics, cx))
            });
    }
    list.into_any_element()
}

/// One account row: id, status, active marker, and the four honest actions.
fn account_row(
    overview: &oxidezap_ipc::AccountOverview,
    active: oxidezap_core::AccountId,
    requested: Option<oxidezap_ipc::AccountStatus>,
    has_control: bool,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> AnyElement {
    let id = overview.id;
    let status = requested.unwrap_or(overview.status);
    let status_text =
        crate::i18n::Language::current().text(crate::app::accounts::status_key(status));
    let switch = entity.clone();
    let reset = entity.clone();
    let remove = entity.clone();

    div()
        .id(SharedString::from(format!("account-row-{id}")))
        .debug_selector(|| format!("account-row-{id}"))
        .flex()
        .items_center()
        .flex_wrap()
        .gap(metrics.space_md())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(metrics.text_meta())
                .child(format!("{} {}", crate::tr!(AccountIdLabel), id)),
        )
        .child(
            div()
                .text_size(metrics.text_meta())
                .text_color(cx.theme().muted_foreground)
                .child(status_text),
        )
        .when(id == active, |this| {
            this.child(
                div()
                    .text_size(metrics.text_micro())
                    .text_color(cx.theme().accent)
                    .child(crate::tr!(Active)),
            )
        })
        .child(
            Button::new(SharedString::from(format!("account-switch-{id}")))
                .label(crate::tr!(Switch))
                .disabled(id == active)
                .on_click(move |_, window, cx| {
                    switch.update(cx, |app, cx| app.switch_account(id, window, cx));
                }),
        )
        .child(
            Button::new(SharedString::from(format!("account-reset-{id}")))
                .label(crate::tr!(Reset))
                .disabled(!has_control || requested.is_some())
                .on_click(move |_, _, cx| {
                    reset.update(cx, |app, cx| app.reset_account(id, cx));
                }),
        )
        .child(
            Button::new(SharedString::from(format!("account-remove-{id}")))
                .label(crate::tr!(Remove))
                .disabled(!has_control || requested.is_some())
                .on_click(move |_, _, cx| {
                    remove.update(cx, |app, cx| app.remove_account(id, cx));
                }),
        )
        .into_any_element()
}

fn notifications(metrics: Metrics, cx: &App) -> AnyElement {
    group(
        label(crate::tr!(Notifications), metrics, cx),
        pending(crate::tr!(NotificationsGuidance), metrics, cx),
        metrics,
    )
    .into_any_element()
}

/// Look up an account by the handle people share.
///
/// A pane of its own rather than a control in the chat list: the answer is a
/// network read with a typed outcome, and the key field has to stay beside the
/// handle when the server asks for it a second time.
fn contacts(
    app: &WhatsAppApp,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> AnyElement {
    group(
        label(crate::tr!(FindByUsername), metrics, cx),
        username_lookup(app, entity, metrics, cx),
        metrics,
    )
    .into_any_element()
}

/// The two fields and the answer they produced.
///
/// Only protocol text is drawn: a verified name comes from the lookup result,
/// and a display name from this account's address book. Nothing here turns a
/// handle into either one.
fn username_lookup(
    app: &WhatsAppApp,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> impl IntoElement + use<> {
    let busy = app.username_lookup_busy();
    let answer = match app.username_lookup_result() {
        Some(UsernameLookupResult::Found(found)) => {
            let mut lines = vec![(crate::tr!(JidLabel).to_string(), found.jid.clone())];
            if let Some(pn) = found.pn_jid.as_deref() {
                lines.push((crate::tr!(PhoneNumber).to_string(), pn.to_string()));
            }
            if let Some(username) = found.username.as_deref() {
                lines.push((crate::tr!(UsernameLabel).to_string(), username.to_string()));
            }
            if let Some(name) = found.name.as_deref() {
                lines.push((crate::tr!(DisplayName).to_string(), name.to_string()));
            }
            if let Some(verified) = found.verified_name.as_deref() {
                lines.push((
                    crate::tr!(VerifiedNameLabel).to_string(),
                    verified.to_string(),
                ));
            }
            card(lines, metrics, cx).into_any_element()
        }
        Some(UsernameLookupResult::KeyRequired { .. }) => {
            pending(crate::tr!(UsernameKeyRequired), metrics, cx).into_any_element()
        }
        Some(UsernameLookupResult::NotFound) => {
            pending(crate::tr!(UsernameNotFound), metrics, cx).into_any_element()
        }
        Some(UsernameLookupResult::InvalidUsername) => {
            pending(crate::tr!(UsernameInvalid), metrics, cx).into_any_element()
        }
        None => pending(crate::tr!(UsernameLookupGuidance), metrics, cx).into_any_element(),
    };

    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(metrics.space_lg())
        .child(answer)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(metrics.space_md())
                .children(app.username_field().map(|field| Input::new(field).w_full()))
                .children(
                    app.username_key_field()
                        .map(|field| Input::new(field).w_full()),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(metrics.space_md())
                .child(
                    Button::new("username-lookup-search")
                        .label(crate::tr!(LookUpUsername))
                        .outline()
                        .disabled(busy)
                        .when(!busy, |button| button.cursor_pointer())
                        .on_click(move |_, _window, cx| {
                            let entity = entity.clone();
                            entity.update(cx, |app, cx| app.lookup_username(cx));
                        }),
                )
                .when(busy, |this| {
                    this.child(
                        div()
                            .text_size(metrics.text_small())
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::tr!(LookingUpUsername)),
                    )
                }),
        )
}

fn privacy(
    app: &WhatsAppApp,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(metrics.space_xxl())
        .child(group(
            label(crate::tr!(Encryption), metrics, cx),
            div()
                .w_full()
                .flex()
                .items_start()
                .gap(metrics.space_lg())
                .p(metrics.space_lg())
                .rounded(metrics.radius_md())
                .bg(cx.theme().secondary)
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    Icon::new(ProductIcon::Lock)
                        .size(metrics.icon_small())
                        .flex_shrink_0()
                        .text_color(cx.theme().success),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(metrics.text_small())
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::tr!(PrivacyGuidance)),
                ),
            metrics,
        ))
        .child(group(
            label(crate::tr!(ReadReceipts), metrics, cx),
            read_receipts(app, entity.clone(), metrics, cx),
            metrics,
        ))
        .child(group(
            label(crate::tr!(PrivacyVisibility), metrics, cx),
            privacy_visibility(app, entity.clone(), metrics, cx),
            metrics,
        ))
        .child(group(
            label(crate::tr!(BlockedContacts), metrics, cx),
            blocklist(app, entity.clone(), metrics, cx),
            metrics,
        ))
        .child(group(
            label(crate::tr!(StartOver), metrics, cx),
            div()
                .flex()
                .flex_col()
                .gap(metrics.space_lg())
                .child(
                    div()
                        .text_size(metrics.text_small())
                        .text_color(cx.theme().muted_foreground)
                        // Named before it is offered: this is the one action
                        // here that cannot be undone.
                        .child(crate::tr!(UnlinkGuidance)),
                )
                .child(
                    div().flex().child(
                        // Outline, not filled: destructive and deliberate, but
                        // not the emphasis of the screen.
                        // Prefixed by the screen it is on, like every other
                        // control here. The logged-out screen has a button
                        // with this action too, and gpui keys interaction
                        // state (hover, press, focus) by element id: two
                        // surfaces drawn in one frame would share it, for the
                        // one action in the app that erases the account.
                        Button::new("settings-pair-again")
                            .label(crate::tr!(ClearDataAndPairAgain))
                            .danger()
                            .outline()
                            .cursor_pointer()
                            .on_click(move |_, window, cx| {
                                entity.update(cx, |app, cx| app.reset_and_pair_again(window, cx));
                            }),
                    ),
                ),
            metrics,
        ))
        .into_any_element()
}

const PRIVACY_CATEGORY_ORDER: [PrivacySettingCategory; 6] = [
    PrivacySettingCategory::LastSeen,
    PrivacySettingCategory::Online,
    PrivacySettingCategory::ProfilePhoto,
    PrivacySettingCategory::About,
    PrivacySettingCategory::GroupAdd,
    PrivacySettingCategory::Messages,
];

/// The only values offered as buttons, in a fixed reading order.
const PRIVACY_VALUE_ORDER: [PrivacySettingValue; 4] = [
    PrivacySettingValue::All,
    PrivacySettingValue::Contacts,
    PrivacySettingValue::None,
    PrivacySettingValue::MatchLastSeen,
];

/// A fixed order, so the six rows appear in the same place on every render.
fn privacy_categories_in_order() -> [PrivacySettingCategory; 6] {
    PRIVACY_CATEGORY_ORDER
}

/// Which values a row may actually offer.
///
/// Both gates matter: a category refuses values it does not understand, and a
/// value such as `contact_blacklist` cannot be sent by the plain setter even
/// where a category would otherwise accept it.
fn settable_privacy_values(category: PrivacySettingCategory) -> Vec<PrivacySettingValue> {
    PRIVACY_VALUE_ORDER
        .into_iter()
        .filter(|value| category.accepts(*value) && value.is_settable())
        .collect()
}

fn privacy_category_label(category: PrivacySettingCategory) -> &'static str {
    match category {
        PrivacySettingCategory::LastSeen => crate::tr!(LastSeenVisibility),
        PrivacySettingCategory::Online => crate::tr!(OnlineVisibility),
        PrivacySettingCategory::ProfilePhoto => crate::tr!(ProfilePhotoVisibility),
        PrivacySettingCategory::About => crate::tr!(AboutVisibility),
        PrivacySettingCategory::GroupAdd => crate::tr!(GroupAddPermission),
        PrivacySettingCategory::Messages => crate::tr!(MessagesPermission),
        // The six rows are the only callers. These two have dedicated paths or
        // no meaning here, and are never drawn as buttons.
        PrivacySettingCategory::ReadReceipts | PrivacySettingCategory::Unknown => "",
    }
}

fn privacy_value_label(value: PrivacySettingValue) -> &'static str {
    match value {
        PrivacySettingValue::All => crate::tr!(Everyone),
        PrivacySettingValue::Contacts => crate::tr!(ContactsOnly),
        PrivacySettingValue::None => crate::tr!(Nobody),
        PrivacySettingValue::MatchLastSeen => crate::tr!(MatchLastSeen),
        // Neither can be offered by the plain setter.
        PrivacySettingValue::ContactBlacklist | PrivacySettingValue::Unknown => "",
    }
}

fn privacy_category_id(category: PrivacySettingCategory) -> &'static str {
    match category {
        PrivacySettingCategory::LastSeen => "last-seen",
        PrivacySettingCategory::Online => "online",
        PrivacySettingCategory::ProfilePhoto => "profile-photo",
        PrivacySettingCategory::About => "about",
        PrivacySettingCategory::GroupAdd => "group-add",
        PrivacySettingCategory::Messages => "messages",
        PrivacySettingCategory::ReadReceipts | PrivacySettingCategory::Unknown => "unsupported",
    }
}

fn privacy_value_id(value: PrivacySettingValue) -> &'static str {
    match value {
        PrivacySettingValue::All => "all",
        PrivacySettingValue::Contacts => "contacts",
        PrivacySettingValue::None => "none",
        PrivacySettingValue::MatchLastSeen => "match-last-seen",
        PrivacySettingValue::ContactBlacklist | PrivacySettingValue::Unknown => "unsupported",
    }
}

/// The six account-wide categories this build can read and set.
///
/// Every row shows the server's answer, or says plainly when it did not give
/// one. A value the build cannot set is reported as unsupported rather than
/// drawn as a selected choice, and a missing category is never guessed from
/// another one. Buttons are disabled while a read or a change is in flight;
/// a click only asks, and the fresh account-wide answer redraws the row.
fn privacy_visibility(
    app: &WhatsAppApp,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> impl IntoElement + use<> {
    let loaded = app.privacy_categories_loaded();
    let unavailable = app.privacy_categories_unavailable();
    let busy = app.privacy_categories_busy();

    let control = if loaded {
        div()
            .w_full()
            .flex()
            .flex_col()
            .rounded(metrics.radius_md())
            .overflow_hidden()
            .bg(cx.theme().secondary)
            .border_1()
            .border_color(cx.theme().border)
            .children(privacy_categories_in_order().into_iter().enumerate().map(
                |(ix, category)| {
                    privacy_category_row(
                        category,
                        app,
                        entity.clone(),
                        busy,
                        ix + 1 == PRIVACY_CATEGORY_ORDER.len(),
                        metrics,
                        cx,
                    )
                },
            ))
            .into_any_element()
    } else if unavailable {
        pending(crate::tr!(PrivacyCategoriesUnavailable), metrics, cx).into_any_element()
    } else {
        pending(crate::tr!(LoadingPrivacyCategories), metrics, cx).into_any_element()
    };

    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(metrics.space_lg())
        .child(
            div()
                .text_size(metrics.text_small())
                .text_color(cx.theme().muted_foreground)
                .child(crate::tr!(PrivacyVisibilityGuidance)),
        )
        .child(control)
}

fn privacy_category_row(
    category: PrivacySettingCategory,
    app: &WhatsAppApp,
    entity: Entity<WhatsAppApp>,
    busy: bool,
    is_last: bool,
    metrics: Metrics,
    cx: &App,
) -> impl IntoElement + use<> {
    let reported = app.privacy_categories().get(category).copied();
    let current = reported.map(|setting| setting.value);
    let status = match reported {
        None => crate::tr!(PrivacyCategoryNotReported),
        Some(setting) if setting.value.is_settable() && category.accepts(setting.value) => "",
        Some(_) => crate::tr!(PrivacyCategoryUnsupported),
    };

    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(metrics.space_md())
        .px(metrics.space_lg())
        .py(metrics.space_lg())
        .when(!is_last, |el| {
            el.border_b_1().border_color(cx.theme().border)
        })
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .gap(metrics.space_lg())
                .child(
                    div()
                        .text_size(metrics.text_secondary())
                        .text_color(cx.theme().foreground)
                        .child(privacy_category_label(category)),
                )
                .when(!status.is_empty(), |el| {
                    el.child(
                        div()
                            .text_size(metrics.text_meta())
                            .text_color(cx.theme().muted_foreground)
                            .child(status),
                    )
                }),
        )
        .child(
            div().flex().flex_wrap().gap(metrics.space_sm()).children(
                settable_privacy_values(category)
                    .into_iter()
                    .map(move |value| {
                        let entity = entity.clone();
                        Button::new(SharedString::from(format!(
                            "privacy-category-{}-{}",
                            privacy_category_id(category),
                            privacy_value_id(value)
                        )))
                        .label(privacy_value_label(value))
                        .outline()
                        .small()
                        .selected(current == Some(value))
                        .disabled(busy)
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |app, cx| {
                                app.set_privacy_category(category, value, cx)
                            });
                        })
                    }),
            ),
        )
}

/// Account-level DM read receipts, from the server and never guessed here.
///
/// The switch is drawn only for the two values this build can read. It is
/// disabled while a change is in flight and is not flipped by the click: the
/// daemon publishes the value it read back from WhatsApp, and that answer is
/// what redraws it. An unrecognised value gets a sentence rather than an
/// unchecked switch, which would claim something the server never said.
fn read_receipts(
    app: &WhatsAppApp,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> impl IntoElement + use<> {
    let value = app.read_receipts();
    let loaded = app.read_receipts_loaded();
    let busy = app.read_receipts_busy();

    let control = match value {
        Some(value) if value.is_known() => div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap(metrics.space_lg())
            .p(metrics.space_lg())
            .rounded(metrics.radius_md())
            .bg(cx.theme().secondary)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_size(metrics.text_secondary())
                    .text_color(cx.theme().foreground)
                    .child(crate::tr!(SendReadReceipts)),
            )
            .child(
                Switch::new("settings-read-receipts")
                    .checked(value == oxidezap_core::PrivacyReadReceipts::All)
                    .disabled(busy)
                    .cursor_pointer()
                    .on_click(move |now, _window, cx| {
                        let entity = entity.clone();
                        let enabled = *now;
                        entity.update(cx, |app, cx| app.set_read_receipts(enabled, cx));
                    }),
            )
            .into_any_element(),
        Some(_) => pending(crate::tr!(ReadReceiptsUnavailable), metrics, cx).into_any_element(),
        None if !loaded => pending(crate::tr!(LoadingReadReceipts), metrics, cx).into_any_element(),
        None => pending(crate::tr!(ReadReceiptsUnavailable), metrics, cx).into_any_element(),
    };

    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(metrics.space_lg())
        .child(
            div()
                .text_size(metrics.text_small())
                .text_color(cx.theme().muted_foreground)
                .child(crate::tr!(ReadReceiptsGuidance)),
        )
        .child(control)
}

/// The account's block list, and the two ways to change it.
///
/// The list is server state: it is read on arrival at this pane and after every
/// change, and the last answer is kept while a new read is on the way. Nothing
/// here guesses at the rows — a failed read says so rather than drawing an
/// empty list over contacts that are still blocked.
fn blocklist(
    app: &WhatsAppApp,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> impl IntoElement + use<> {
    let entries = app.blocklist_entries();
    let loaded = app.blocklist_loaded();
    let busy = app.blocklist_busy();

    let rows = if !loaded {
        pending(crate::tr!(LoadingBlocklist), metrics, cx).into_any_element()
    } else if entries.is_empty() {
        pending(crate::tr!(NoBlockedContacts), metrics, cx).into_any_element()
    } else {
        div()
            .w_full()
            .flex()
            .flex_col()
            .rounded(metrics.radius_md())
            .overflow_hidden()
            .bg(cx.theme().secondary)
            .border_1()
            .border_color(cx.theme().border)
            .children(entries.iter().enumerate().map(|(ix, contact)| {
                let is_last = ix + 1 == entries.len();
                let jid = contact.jid.clone();
                let label = contact.label().to_string();
                let row_entity = entity.clone();
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(metrics.space_lg())
                    .px(metrics.space_lg())
                    .py(metrics.space_lg())
                    .when(!is_last, |el| {
                        el.border_b_1().border_color(cx.theme().border)
                    })
                    .child(
                        parts::one_line()
                            .flex_1()
                            .min_w_0()
                            .text_size(metrics.text_secondary())
                            .text_color(cx.theme().foreground)
                            .child(label),
                    )
                    .child(
                        Button::new(format!("unblock-{jid}"))
                            .label(crate::tr!(Unblock))
                            .outline()
                            .cursor_pointer()
                            .on_click(move |_, _window, cx| {
                                let jid = jid.clone();
                                row_entity.update(cx, |app, cx| app.unblock_contact(jid, cx));
                            }),
                    )
                    .into_any_element()
            }))
            .into_any_element()
    };

    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(metrics.space_lg())
        .child(
            div()
                .text_size(metrics.text_small())
                .text_color(cx.theme().muted_foreground)
                .child(crate::tr!(BlockedContactsGuidance)),
        )
        .child(rows)
        .child(
            div()
                .flex()
                .items_center()
                .gap(metrics.space_md())
                .children(
                    app.blocklist_field()
                        .map(|field| Input::new(field).w_full()),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(metrics.space_md())
                .child({
                    let refresh_entity = entity.clone();
                    Button::new("blocklist-refresh")
                        .label(crate::tr!(RefreshBlocklist))
                        .outline()
                        .disabled(busy)
                        .when(!busy, |button| button.cursor_pointer())
                        .on_click(move |_, _window, cx| {
                            refresh_entity.update(cx, |app, cx| app.refresh_blocklist(cx));
                        })
                })
                .child({
                    let block_entity = entity.clone();
                    Button::new("blocklist-block")
                        .label(crate::tr!(BlockContact))
                        .cursor_pointer()
                        .on_click(move |_, window, cx| {
                            block_entity.update(cx, |app, cx| app.block_contact(window, cx));
                        })
                }),
        )
}

fn storage(
    app: &WhatsAppApp,
    entity: Entity<WhatsAppApp>,
    metrics: Metrics,
    cx: &App,
) -> AnyElement {
    let usage = app.storage_usage(cx);
    let can_clear_media = usage.is_some_and(|u| u.media_files > 0);

    div()
        .flex()
        .flex_col()
        .gap(metrics.space_xxl())
        .child(group(
            label(crate::tr!(OnDisk), metrics, cx),
            card(
                vec![
                    (
                        // The shared store: every local account lives in one
                        // database file, and SQLite cannot report one account's
                        // share of it. Said as the file it is rather than as
                        // this account's private usage.
                        crate::tr!(MessageStoreShared).to_string(),
                        // Until the first answer arrives. The daemon measures,
                        // and it is another process: there is a frame or two
                        // where the honest thing to show is that nobody has
                        // counted yet.
                        usage.map_or_else(
                            || crate::tr!(Measuring).to_string(),
                            |u| format_bytes(u.database_bytes),
                        ),
                    ),
                    (
                        crate::tr!(DownloadedMedia).to_string(),
                        usage.map_or_else(
                            || crate::tr!(Measuring).to_string(),
                            |u| {
                                format!(
                                    "{} · {}",
                                    format_bytes(u.media_bytes),
                                    files(u.media_files)
                                )
                            },
                        ),
                    ),
                ],
                metrics,
                cx,
            ),
            metrics,
        ))
        // A jump list over the store's media rows, opened here rather than
        // from a gallery: it is bounded rows to click, not a cache view.
        .child(group(
            label(crate::tr!(MediaMessages), metrics, cx),
            div()
                .flex()
                .flex_col()
                .gap(metrics.space_lg())
                .child(
                    div()
                        .text_size(metrics.text_small())
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::tr!(MediaListGuidance)),
                )
                .child({
                    let media_entity = entity.clone();
                    div().flex().child(
                        Button::new("open-media-messages")
                            .label(crate::tr!(OpenMediaMessages))
                            .outline()
                            .cursor_pointer()
                            .on_click(move |_, _window, cx| {
                                media_entity.update(cx, |app, cx| app.open_media_messages(cx));
                            }),
                    )
                }),
            metrics,
        ))
        .child(group(
            label(crate::tr!(MediaCache), metrics, cx),
            div()
                .flex()
                .flex_col()
                .gap(metrics.space_lg())
                .child(
                    div()
                        .text_size(metrics.text_small())
                        .text_color(cx.theme().muted_foreground)
                        // What it costs, so the button is a decision rather
                        // than a dare: the history is untouched and every
                        // message keeps the means to fetch its media again.
                        .child(crate::tr!(CacheGuidance)),
                )
                .child(
                    div().flex().child(
                        Button::new("clear-media-cache")
                            .label(crate::tr!(ClearCachedMedia))
                            .outline()
                            .disabled(!can_clear_media)
                            .when(can_clear_media, |button| button.cursor_pointer())
                            .on_click(move |_, _window, cx| {
                                entity.update(cx, |app, cx| app.clear_media_cache(cx));
                            }),
                    ),
                ),
            metrics,
        ))
        .into_any_element()
}

/// A size a person can read. Binary units, because that is what a filesystem
/// reports and a number that disagrees with `du` invites a bug report.
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    // One decimal below ten, none above: "1.4 MiB" is useful, "847.3 MiB" is
    // three digits of noise.
    if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

fn files(count: u64) -> String {
    if count == 1 {
        crate::tr!(OneFile).to_string()
    } else {
        crate::tr!(files, count)
    }
}

/// Diagnostics: how loud the client is, and what is drawing it.
///
/// The level is a control rather than a line of text, which is the whole of
/// this section's reason to exist. It used to report `RUST_LOG` — a variable
/// a person can only set by restarting the client from a terminal they may
/// not have, and one a page has never had at all. What is drawn now takes
/// effect in this window and in the daemon at once, and is remembered.
fn advanced(entity: Entity<WhatsAppApp>, metrics: Metrics, cx: &App) -> AnyElement {
    let diagnostics = diagnostics(entity.clone(), metrics, cx);
    if !crate::platform::background::is_configurable() {
        return diagnostics;
    }
    let keep = crate::platform::background::keep_running();
    div()
        .flex()
        .flex_col()
        .gap(metrics.space_xl())
        .child(group(
            label(crate::tr!(Background), metrics, cx),
            div()
                .flex()
                .flex_col()
                .gap(metrics.space_md())
                .child(
                    gpui_component::checkbox::Checkbox::new("keep-running")
                        .label(crate::tr!(KeepRunningInBackground))
                        .checked(keep)
                        .on_click({
                            let entity = entity.clone();
                            move |checked, _, cx| {
                                entity.update(cx, |app, cx| app.set_keep_running(*checked, cx));
                            }
                        }),
                )
                .child(
                    div()
                        .text_size(metrics.text_meta())
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::tr!(KeepRunningNote)),
                ),
            metrics,
        ))
        .child(diagnostics)
        .into_any_element()
}

fn diagnostics(entity: Entity<WhatsAppApp>, metrics: Metrics, cx: &App) -> AnyElement {
    let active = oxidezap_logging::current();
    // A level given for this run from outside — `RUST_LOG` on a desktop,
    // `?log=` in a page — is what the process *started* at. Said rather than
    // hidden: without it, a stored choice that a launch argument overrode
    // looks like a control that did not work.
    let forced = oxidezap_logging::forced();
    let kept = oxidezap_logging::location();

    group(
        label(crate::tr!(Diagnostics), metrics, cx),
        div()
            .flex()
            .flex_col()
            .gap(metrics.space_lg())
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(metrics.space_md())
                    // `Button`s rather than styled `div`s, for the reason the
                    // density control uses them: picking a level changes
                    // application state, and a `div` carries no focus handle
                    // and no keyboard activation.
                    .children(LogLevel::ALL.into_iter().map(|level| {
                        let entity = entity.clone();
                        Button::new(SharedString::from(format!("log-level-{}", level.id())))
                            .label(level.label())
                            .outline()
                            .selected(level == active)
                            .px(metrics.space_xl())
                            .py(metrics.space_md())
                            .rounded(metrics.radius_md())
                            .text_size(metrics.text_small())
                            .cursor_pointer()
                            .on_click(move |_, _window, cx| {
                                entity.update(cx, |app, cx| app.set_log_level(level, cx));
                            })
                    })),
            )
            .child(
                div()
                    .text_size(metrics.text_meta())
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::i18n::Language::current().log_note(active)),
            )
            .child(card(
                vec![
                    (crate::tr!(LogLevel).to_string(), active.label().to_string()),
                    (
                        crate::tr!(KeptIn).to_string(),
                        kept.unwrap_or_else(|| crate::tr!(NowhereItLastsForThisRun).to_string()),
                    ),
                    (crate::tr!(Renderer).to_string(), "GPUI".to_string()),
                ],
                metrics,
                cx,
            ))
            .when_some(forced, |el, forced| {
                el.child(
                    div()
                        .text_size(metrics.text_meta())
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "{} asked for {forced} when this started; the level above is \
                             what is in force now.",
                            oxidezap_logging::forced_by(),
                        )),
                )
            }),
        metrics,
    )
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use oxidezap_core::{PrivacySettingCategory as Category, PrivacySettingValue as Value};

    use super::{
        format_bytes, privacy_categories_in_order, privacy_category_label, privacy_value_label,
        settable_privacy_values,
    };

    /// The unit is binary and says so. A settings row that reports 1000-based
    /// "MB" over a filesystem that counts in 1024s is a number nobody can
    /// check against `du`, and the second formatter this file used to carry
    /// did exactly that — in KB, tested, and reachable from nothing.
    #[test]
    fn sizes_scale_to_a_readable_binary_unit() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(900), "900 B");
        assert_eq!(format_bytes(2048), "2.0 KiB");
        assert_eq!(format_bytes(10_485_760), "10 MiB");
    }

    /// One decimal below ten and none above: "1.4 MiB" is useful, "847.3 MiB"
    /// is three digits of noise.
    #[test]
    fn the_decimal_goes_where_it_carries_information() {
        assert_eq!(format_bytes(1_572_864), "1.5 MiB");
        assert_eq!(format_bytes(245_366_784), "234 MiB");
    }

    /// The pane is a fixed list: neither the dedicated receipts setting nor an
    /// unrecognised category may join it, and the order never drifts with a
    /// server's answer.
    #[test]
    fn privacy_rows_are_the_six_settable_categories_in_order() {
        assert_eq!(
            privacy_categories_in_order(),
            [
                Category::LastSeen,
                Category::Online,
                Category::ProfilePhoto,
                Category::About,
                Category::GroupAdd,
                Category::Messages,
            ]
        );
        for category in privacy_categories_in_order() {
            assert!(!privacy_category_label(category).is_empty());
        }
    }

    /// Both gates are applied to one fixed source. In particular a category
    /// never gets `none` where upstream only permits `all` and `contacts`, and
    /// neither a recognised but unsendable value nor an unknown one becomes a
    /// button that looks like a choice.
    #[test]
    fn privacy_values_are_filtered_by_category_and_setter() {
        assert_eq!(
            settable_privacy_values(Category::LastSeen),
            vec![Value::All, Value::Contacts, Value::None]
        );
        assert_eq!(
            settable_privacy_values(Category::Online),
            vec![Value::All, Value::MatchLastSeen]
        );
        assert_eq!(
            settable_privacy_values(Category::ProfilePhoto),
            vec![Value::All, Value::Contacts, Value::None]
        );
        assert_eq!(
            settable_privacy_values(Category::About),
            vec![Value::All, Value::Contacts, Value::None]
        );
        assert_eq!(
            settable_privacy_values(Category::GroupAdd),
            vec![Value::All, Value::Contacts, Value::None]
        );
        assert_eq!(
            settable_privacy_values(Category::Messages),
            vec![Value::All, Value::Contacts]
        );
        for category in privacy_categories_in_order() {
            for value in settable_privacy_values(category) {
                assert!(category.accepts(value) && value.is_settable());
                assert!(!privacy_value_label(value).is_empty());
            }
        }
    }
}
