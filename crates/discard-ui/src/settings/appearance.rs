use super::{SettingsView, general::apply_change};
use crate::{appearance, problems::theme_problem_lines, shared::Shared};
use discard_core::ghostty::{
    copy::copy_to_yours,
    lookup::Origin,
    map::swatches,
    resolve::ThemeSource,
    spec::{Appearance, ThemeSpec},
};
use discard_core::settings::Change;
use discard_core::settings_page::*;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectGroup, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

type Items = SearchableVec<SelectGroup<String>>;
type Picker = Entity<SelectState<Items>>;

pub(super) struct Pickers {
    light: Picker,
    dark: Picker,
    last: String,
    refresh: bool,
    _events: Vec<Subscription>,
}

fn items(shared: &Shared) -> Items {
    let (dirs, _) = appearance::theme_env(&shared.discard_home);
    let list = dirs.list();
    SearchableVec::new(
        [
            (Origin::Yours, "Yours"),
            (Origin::Ghostty, "Ghostty"),
            (Origin::Bundled, "Bundled"),
        ]
        .into_iter()
        .map(|(origin, title)| {
            SelectGroup::new(title).items(
                list.iter()
                    .filter(|(_, o)| *o == origin)
                    .map(|(n, _)| n.clone()),
            )
        })
        .collect::<Vec<_>>(),
    )
}

impl Pickers {
    pub fn new(shared: &Shared, window: &mut Window, cx: &mut Context<SettingsView>) -> Self {
        let light = cx.new(|cx| SelectState::new(items(shared), None, window, cx).searchable(true));
        let dark = cx.new(|cx| SelectState::new(items(shared), None, window, cx).searchable(true));
        let events = [(&light, true), (&dark, false)]
            .into_iter()
            .map(|(picker, is_light)| {
                cx.subscribe_in(
                    picker,
                    window,
                    move |view, _, event: &SelectEvent<Items>, _, cx| {
                        if let SelectEvent::Confirm(Some(name)) = event {
                            let settings = view
                                .shared
                                .settings
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .settings()
                                .appearance
                                .clone();
                            let spec = ThemeSpec::parse(&settings.theme)
                                .unwrap_or_else(|| ThemeSpec::Single(name.clone()));
                            let next = if matches!(spec, ThemeSpec::Single(_)) {
                                ThemeSpec::Single(name.clone())
                            } else if is_light {
                                spec.with_light(name)
                            } else {
                                spec.with_dark(name)
                            };
                            apply(view, Change::Theme(next.to_value()), cx);
                        }
                    },
                )
            })
            .collect();
        Self {
            light,
            dark,
            last: String::new(),
            refresh: false,
            _events: events,
        }
    }
}

fn apply(view: &mut SettingsView, change: Change, cx: &mut Context<SettingsView>) {
    let result = apply_change(
        &mut view
            .shared
            .settings
            .lock()
            .unwrap_or_else(|p| p.into_inner()),
        change,
    );
    view.appearance_error = result.err();
    if view.appearance_error.is_some() {
        view.pickers.last.clear();
    }
    cx.notify();
}

fn open_editor(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/c", "start", ""]);
        c
    };
    let status = command.arg(path).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "Editor command failed: {status}"
        )))
    }
}

/// A section label, in the weight the General page uses.
fn label(text: impl Into<SharedString>) -> Div {
    div().font_weight(FontWeight::SEMIBOLD).child(text.into())
}

/// A muted line under a label.
fn hint(text: impl Into<SharedString>, color: Hsla) -> Div {
    div().text_xs().text_color(color).child(text.into())
}

/// One palette color, as a small square.
fn swatch(color: discard_core::ghostty::color::Rgb, border: Hsla) -> Div {
    let value = (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b);
    div()
        .size(px(22.))
        .flex_none()
        .rounded_sm()
        .border_1()
        .border_color(border)
        .bg(rgb(value))
}

/// Where the active theme came from, in words.
fn origin_name(origin: Origin) -> &'static str {
    match origin {
        Origin::Yours => "Yours",
        Origin::Ghostty => "Ghostty",
        Origin::Bundled => "Bundled",
    }
}

pub(super) fn render(
    view: &mut SettingsView,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) -> AnyElement {
    let settings = view
        .shared
        .settings
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .settings()
        .appearance
        .clone();
    let (name, problems, colors) = {
        let state = view
            .shared
            .appearance
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        (
            state.theme_name().to_string(),
            theme_problem_lines(state.problems()),
            swatches(state.colors()),
        )
    };
    let spec = ThemeSpec::parse(&settings.theme).unwrap_or_else(|| ThemeSpec::Single(name.clone()));
    if view.pickers.refresh {
        view.pickers
            .light
            .update(cx, |p, cx| p.set_items(items(&view.shared), window, cx));
        view.pickers
            .dark
            .update(cx, |p, cx| p.set_items(items(&view.shared), window, cx));
        view.pickers.last.clear();
        view.pickers.refresh = false;
    }
    if view.pickers.last != settings.theme {
        view.pickers.light.update(cx, |p, cx| {
            p.set_selected_value(&spec.name_for(Appearance::Light).to_string(), window, cx)
        });
        view.pickers.dark.update(cx, |p, cx| {
            p.set_selected_value(&spec.name_for(Appearance::Dark).to_string(), window, cx)
        });
        view.pickers.last = settings.theme.clone();
    }

    let (dirs, configs) = appearance::theme_env(&view.shared.discard_home);
    let found = dirs.find(&name);
    let origin = found.as_ref().map(|f| origin_name(f.origin));
    let file = match &found {
        Some(f) => f
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "Built into Discard".into()),
        None => "No theme file found".into(),
    };
    let config = configs
        .into_iter()
        .find(|p| p.is_file())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "No Ghostty config found".into());
    let sources = view.shared.plugins.theme_sources();

    let theme = cx.theme();
    let (muted, danger, border) = (theme.muted_foreground, theme.danger, theme.border);
    let custom = settings.theme_source == ThemeSource::Custom;
    let single = matches!(spec, ThemeSpec::Single(_));
    let source_hint = match settings.theme_source {
        ThemeSource::Default => "Catppuccin, light or dark with your system",
        ThemeSource::Ghostty => "Use the theme in your Ghostty config",
        ThemeSource::Custom => "Pick the themes below",
    };

    let source_buttons = h_flex().gap_2().children(
        [
            (ThemeSource::Default, "Default"),
            (ThemeSource::Ghostty, "Ghostty"),
            (ThemeSource::Custom, "Custom"),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (source, text))| {
            Button::new(("theme-source", i))
                .small()
                .label(text)
                .when(settings.theme_source == source, |b| b.primary())
                .when(settings.theme_source != source, |b| b.outline())
                .on_click(
                    cx.listener(move |view, _, _, cx| apply(view, Change::ThemeSource(source), cx)),
                )
        }),
    );

    let one = cx.entity();
    let one_switch = Switch::new("one-theme")
        .checked(single)
        .accessibility_label(LABEL_ONE_THEME)
        .on_click(move |on, _, cx| {
            one.update(cx, |view, cx| {
                let value = view
                    .shared
                    .settings
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .settings()
                    .appearance
                    .theme
                    .clone();
                if let Some(spec) = ThemeSpec::parse(&value) {
                    let next = if *on {
                        spec.as_single(appearance::system_appearance(cx.window_appearance()))
                    } else {
                        spec.as_pair()
                    };
                    apply(view, Change::Theme(next.to_value()), cx);
                }
            });
        });
    let picker = |title: &'static str, state: &Picker| {
        v_flex()
            .gap_1()
            .child(div().text_sm().text_color(muted).child(title))
            .child(Select::new(state).disabled(!custom).w(px(240.)))
    };
    let pickers = h_flex()
        .gap_3()
        .items_end()
        .when(single, |r| r.child(picker("Theme", &view.pickers.light)))
        .when(!single, |r| {
            r.child(picker(LABEL_LIGHT_THEME, &view.pickers.light))
                .child(picker(LABEL_DARK_THEME, &view.pickers.dark))
        });

    let copy_name = name.clone();
    let copy = Button::new("edit-copy")
        .small()
        .outline()
        .label(LABEL_EDIT_COPY)
        .on_click(cx.listener(move |view, _, _, cx| {
            let (dirs, _) = appearance::theme_env(&view.shared.discard_home);
            view.appearance_error = copy_to_yours(&dirs, &copy_name)
                .map_err(|e| e.to_string())
                .and_then(|path| open_editor(&path).map_err(|e| e.to_string()))
                .err();
            view.pickers.refresh = true;
            cx.notify();
        }));

    let ignore = cx.entity();
    let ignore_switch = Switch::new("ignore-plugin-themes")
        .checked(settings.ignore_plugin_themes)
        .accessibility_label(LABEL_IGNORE_PLUGIN_THEMES)
        .on_click(move |on, _, cx| {
            ignore.update(cx, |view, cx| {
                apply(view, Change::IgnorePluginThemes(*on), cx)
            });
        });
    let overrides = if settings.ignore_plugin_themes {
        hint("Plugin themes are off", muted).into_any_element()
    } else if sources.is_empty() {
        hint("No plugin changes the theme", muted).into_any_element()
    } else {
        v_flex()
            .gap_0p5()
            .text_sm()
            .children(sources.into_iter().map(|(token, plugin)| {
                h_flex()
                    .gap_2()
                    .child(div().child(token))
                    .child(div().text_color(muted).child(format!("from {plugin}")))
            }))
            .into_any_element()
    };

    let mut palette = colors;
    let special = palette.split_off(16);
    let palette_rows = [0, 8].map(|start| {
        h_flex().gap_1().children(
            palette[start..start + 8]
                .iter()
                .map(|(_, c)| swatch(*c, border)),
        )
    });
    let special_swatches = h_flex()
        .gap_3()
        .children(special.into_iter().map(|(text, c)| {
            h_flex()
                .gap_1p5()
                .items_center()
                .child(swatch(c, border))
                .child(div().text_xs().text_color(muted).child(text))
        }));

    v_flex()
        .gap_5()
        .when(!problems.is_empty(), |v| {
            v.child(
                v_flex()
                    .gap_1()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(danger)
                    .text_sm()
                    .child(label("Problems with the theme"))
                    .children(problems),
            )
        })
        .when_some(view.appearance_error.clone(), |v, e| {
            v.child(
                div()
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .border_color(danger)
                    .text_sm()
                    .child(e),
            )
        })
        .child(
            v_flex()
                .gap_1()
                .child(label(LABEL_THEME_SOURCE))
                .child(source_buttons)
                .child(hint(source_hint, muted)),
        )
        .when(custom, |v| {
            v.child(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .child(label(LABEL_ONE_THEME))
                            .child(one_switch),
                    )
                    .child(pickers)
                    .child(hint(
                        if single {
                            "The same theme in light and dark"
                        } else {
                            "Discard switches with your system's light and dark setting"
                        },
                        muted,
                    )),
            )
        })
        .child(
            v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(label(name.clone()))
                        .when_some(origin, |r, o| r.child(hint(o, muted)))
                        .child(div().flex_1())
                        .child(copy),
                )
                .child(
                    v_flex()
                        .gap_0p5()
                        .child(hint(format!("{LABEL_ACTIVE_THEME_FILE}: {file}"), muted).min_w_0())
                        .child(hint(format!("{LABEL_GHOSTTY_CONFIG}: {config}"), muted).min_w_0()),
                )
                .child(
                    v_flex()
                        .gap_1p5()
                        .pt_1()
                        .child(label(LABEL_SWATCHES))
                        .child(v_flex().gap_1().children(palette_rows))
                        .child(special_swatches),
                ),
        )
        .child(
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(label(LABEL_IGNORE_PLUGIN_THEMES))
                        .child(ignore_switch),
                )
                .child(hint(
                    "Keep the theme colors above, whatever plugins ask for",
                    muted,
                )),
        )
        .child(
            v_flex()
                .gap_1()
                .child(label(LABEL_PLUGIN_OVERRIDES))
                .child(overrides),
        )
        .into_any_element()
}
