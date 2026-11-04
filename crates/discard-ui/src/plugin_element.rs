//! Draws a plugin's element tree with GPUI components.

use discard_lua::element::{Element, Node, Prop, Tag};
use gpui_kit::component::avatar::Avatar;
use gpui_kit::component::badge::Badge;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::text::markdown;
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::*;
use std::rc::Rc;

/// Builds the click handler for a plugin action id.
pub type ActionFactory = Rc<dyn Fn(&str) -> Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>>;

fn token_color(name: &str, cx: &App) -> Option<Hsla> {
    let c = &cx.theme().colors;
    Some(match name {
        "background" => c.background,
        "foreground" => c.foreground,
        "sidebar" => c.sidebar,
        "sidebar_foreground" => c.sidebar_foreground,
        "sidebar_accent" => c.sidebar_accent,
        "primary" => c.primary,
        "primary_foreground" => c.primary_foreground,
        "muted" => c.muted,
        "muted_foreground" => c.muted_foreground,
        "border" => c.border,
        "accent" => c.accent,
        "success" => c.success,
        "warning" => c.warning,
        "danger" => c.danger,
        _ => return None,
    })
}

fn num(e: &Element, key: &str) -> Option<f32> {
    match e.props.get(key) {
        Some(Prop::Number(n)) => Some(*n as f32),
        _ => None,
    }
}

fn text_prop<'a>(e: &'a Element, key: &str) -> Option<&'a str> {
    match e.props.get(key) {
        Some(Prop::Text(t)) => Some(t),
        _ => None,
    }
}

fn flag(e: &Element, key: &str) -> bool {
    matches!(e.props.get(key), Some(Prop::Bool(true)))
}

/// All text children joined.
fn plain_text(e: &Element) -> String {
    e.children
        .iter()
        .map(|n| match n {
            Node::Text(t) => t.clone(),
            Node::Element(inner) => plain_text(inner),
        })
        .collect()
}

fn container<T: Styled + InteractiveElement + ParentElement>(mut d: T, e: &Element, cx: &App) -> T {
    if let Some(g) = num(e, "gap") {
        d = d.gap(px(g));
    }
    if let Some(p) = num(e, "padding") {
        d = d.p(px(p));
    }
    if let Some(p) = num(e, "padding_x") {
        d = d.px(px(p));
    }
    if let Some(p) = num(e, "padding_y") {
        d = d.py(px(p));
    }
    if let Some(w) = num(e, "width") {
        d = d.w(px(w)).flex_none();
    }
    if flag(e, "grow") {
        d = d.flex_1();
    }
    if flag(e, "height_full") {
        d = d.h_full();
    }
    match text_prop(e, "align") {
        Some("center") => d = d.items_center(),
        Some("end") => d = d.items_end(),
        Some("start") => d = d.items_start(),
        _ => {}
    }
    let border = cx.theme().colors.border;
    if flag(e, "border_bottom") {
        d = d.border_b_1().border_color(border);
    }
    if flag(e, "border_top") {
        d = d.border_t_1().border_color(border);
    }
    if flag(e, "border_left") {
        d = d.border_l_1().border_color(border);
    }
    if flag(e, "border_right") {
        d = d.border_r_1().border_color(border);
    }
    if let Some(c) = text_prop(e, "bg").and_then(|t| token_color(t, cx)) {
        d = d.bg(c);
    }
    d
}

/// Draw an avatar with a presence dot and a sidebar-colored ring.
pub(crate) fn avatar_with_status(avatar: Avatar, color: Hsla, dot_size: Pixels, cx: &App) -> impl IntoElement {
    div().relative().child(avatar).child(
        // Sit past the corner so the dot does not cover the initials.
        div().absolute().bottom(px(-3.)).right(px(-3.)).size(dot_size).rounded_full()
            .border_2().border_color(cx.theme().sidebar).bg(color),
    )
}

pub fn render_element(e: &Element, plugin: &str, actions: &ActionFactory, dot_size: Pixels, cx: &App) -> AnyElement {
    let children = |actions: &ActionFactory| -> Vec<AnyElement> {
        e.children
            .iter()
            .map(|n| match n {
                Node::Text(t) => div().child(t.clone()).into_any_element(),
                Node::Element(inner) => render_element(inner, plugin, actions, dot_size, cx),
            })
            .collect()
    };
    let color = text_prop(e, "color").and_then(|t| token_color(t, cx));
    match e.tag {
        Tag::Row => {
            let d = container(h_flex(), e, cx).children(children(actions));
            match color {
                Some(c) => d.text_color(c).into_any_element(),
                None => d.into_any_element(),
            }
        }
        Tag::Column => {
            let d = container(v_flex(), e, cx).children(children(actions));
            match color {
                Some(c) => d.text_color(c).into_any_element(),
                None => d.into_any_element(),
            }
        }
        Tag::Text => {
            let mut d = div().child(plain_text(e));
            d = match text_prop(e, "size") {
                Some("xs") => d.text_xs(),
                Some("sm") => d.text_sm(),
                Some("lg") => d.text_lg(),
                _ => d,
            };
            d = match text_prop(e, "weight") {
                Some("bold") => d.font_weight(FontWeight::BOLD),
                Some("semibold") => d.font_weight(FontWeight::SEMIBOLD),
                _ => d,
            };
            match color {
                Some(c) => d.text_color(c).into_any_element(),
                None => d.into_any_element(),
            }
        }
        Tag::Markdown => div().child(markdown(plain_text(e))).into_any_element(),
        Tag::Button => {
            let mut b = Button::new(SharedString::from(format!("{plugin}:{}", text_prop(e, "on_click").unwrap_or("")))).label(plain_text(e)).small();
            b = match text_prop(e, "variant") {
                Some("primary") => b.primary(),
                Some("danger") => b.danger(),
                Some("success") => b.success(),
                Some("outline") => b.outline(),
                _ => b,
            };
            if let Some(action) = text_prop(e, "on_click") {
                b = b.on_click(actions(action));
            }
            b.into_any_element()
        }
        Tag::Avatar => {
            let mut a = Avatar::new().name(text_prop(e, "name").unwrap_or("?").to_string());
            a = match text_prop(e, "size") {
                Some("small") => a.small(),
                Some("large") => a.large(),
                _ => a,
            };
            match text_prop(e, "status").and_then(|name| token_color(name, cx)) {
                Some(color) => avatar_with_status(a, color, dot_size, cx).into_any_element(),
                None => a.into_any_element(),
            }
        }
        Tag::Badge => Badge::new().count(num(e, "count").unwrap_or(0.0).max(0.0) as usize).into_any_element(),
        Tag::Divider => div().h(px(1.)).w_full().bg(cx.theme().colors.border).into_any_element(),
        Tag::Spacer => div().flex_1().into_any_element(),
        Tag::Dot => {
            let c = color.unwrap_or(cx.theme().colors.muted_foreground);
            div().size_2().rounded_full().bg(c).into_any_element()
        }
    }
}
