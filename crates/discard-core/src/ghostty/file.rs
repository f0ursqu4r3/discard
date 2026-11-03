//! Read a Ghostty config or theme file. Both use `key = value` lines.
//! Discard reads only the color keys and `theme`; it skips other keys.

use super::color::Rgb;

/// The Ghostty colors Discard uses. `None` means "not set".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeColors {
    /// Palette entries 0-15. Entries 16-255 are not used.
    pub palette: [Option<Rgb>; 16],
    pub background: Option<Rgb>,
    pub foreground: Option<Rgb>,
    pub cursor_color: Option<Rgb>,
    pub cursor_text: Option<Rgb>,
    pub selection_background: Option<Rgb>,
    pub selection_foreground: Option<Rgb>,
    pub split_divider_color: Option<Rgb>,
}

impl ThemeColors {
    /// Every color set in `over` replaces the color here.
    pub fn overlaid_with(&self, over: &ThemeColors) -> ThemeColors {
        let pick = |base: Option<Rgb>, top: Option<Rgb>| top.or(base);
        let mut palette = self.palette;
        for (slot, top) in palette.iter_mut().zip(over.palette) {
            *slot = pick(*slot, top);
        }
        ThemeColors {
            palette,
            background: pick(self.background, over.background),
            foreground: pick(self.foreground, over.foreground),
            cursor_color: pick(self.cursor_color, over.cursor_color),
            cursor_text: pick(self.cursor_text, over.cursor_text),
            selection_background: pick(self.selection_background, over.selection_background),
            selection_foreground: pick(self.selection_foreground, over.selection_foreground),
            split_divider_color: pick(self.split_divider_color, over.split_divider_color),
        }
    }
}

/// One bad line in a file. `line` starts at 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIssue {
    pub line: usize,
    pub message: String,
}

/// The result of reading one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedFile {
    pub colors: ThemeColors,
    /// The raw value of the last `theme` line.
    pub theme: Option<String>,
    pub issues: Vec<FileIssue>,
}

/// Read a config or theme file. A bad line becomes an issue and the rest
/// of the file still applies.
pub fn parse(text: &str) -> ParsedFile {
    let mut parsed = ParsedFile::default();
    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            parsed.issues.push(FileIssue {
                line,
                message: format!("expected `key = value`, found `{trimmed}`"),
            });
            continue;
        };
        let key = key.trim();
        let value = unquote(value.trim());
        if let Err(message) = apply_line(&mut parsed, key, value) {
            parsed.issues.push(FileIssue { line, message });
        }
    }
    parsed
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(value)
}

/// Apply one `key = value` line. Unknown keys are not an error.
fn apply_line(parsed: &mut ParsedFile, key: &str, value: &str) -> Result<(), String> {
    let colors = &mut parsed.colors;
    let slot = match key {
        "theme" => {
            parsed.theme = (!value.is_empty()).then(|| value.to_string());
            return Ok(());
        }
        "palette" => return apply_palette(colors, value),
        "background" => &mut colors.background,
        "foreground" => &mut colors.foreground,
        "cursor-color" => &mut colors.cursor_color,
        "cursor-text" => &mut colors.cursor_text,
        "selection-background" => &mut colors.selection_background,
        "selection-foreground" => &mut colors.selection_foreground,
        "split-divider-color" => &mut colors.split_divider_color,
        _ => return Ok(()),
    };
    *slot = parse_color_value(key, value)?;
    Ok(())
}

/// An empty value clears the color. `cell-foreground` and `cell-background`
/// are terminal-only modes, so they count as "not set".
fn parse_color_value(key: &str, value: &str) -> Result<Option<Rgb>, String> {
    if value.is_empty() || value == "cell-foreground" || value == "cell-background" {
        return Ok(None);
    }
    Rgb::parse_ghostty(value)
        .map(Some)
        .ok_or_else(|| format!("`{key}`: `{value}` is not a color"))
}

/// `palette = N=#rrggbb`. Entries 16-255 are valid but unused.
fn apply_palette(colors: &mut ThemeColors, value: &str) -> Result<(), String> {
    let (index, color) = value
        .split_once('=')
        .ok_or_else(|| format!("`palette`: expected `N=color`, found `{value}`"))?;
    let index: usize = index
        .trim()
        .parse()
        .ok()
        .filter(|n| *n <= 255)
        .ok_or_else(|| format!("`palette`: `{}` is not an index from 0 to 255", index.trim()))?;
    let color = parse_color_value("palette", color.trim())?;
    if let Some(slot) = colors.palette.get_mut(index) {
        *slot = color;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    const SAMPLE: &str = "\
palette = 0=#1b2123
palette = 1=#dc3c3c
palette = 15=#cee5e0
background = #0a0f04
foreground = #fefbdc
cursor-color = #c4bba1
cursor-text = #050413
selection-background = #a89fa6
selection-foreground = #040405
";

    #[test]
    fn reads_every_color_key() {
        let parsed = parse(SAMPLE);
        assert!(parsed.issues.is_empty(), "{:?}", parsed.issues);
        let c = parsed.colors;
        assert_eq!(c.palette[0], Some(rgb(0x1b, 0x21, 0x23)));
        assert_eq!(c.palette[1], Some(rgb(0xdc, 0x3c, 0x3c)));
        assert_eq!(c.palette[2], None);
        assert_eq!(c.palette[15], Some(rgb(0xce, 0xe5, 0xe0)));
        assert_eq!(c.background, Some(rgb(0x0a, 0x0f, 0x04)));
        assert_eq!(c.foreground, Some(rgb(0xfe, 0xfb, 0xdc)));
        assert_eq!(c.cursor_color, Some(rgb(0xc4, 0xbb, 0xa1)));
        assert_eq!(c.cursor_text, Some(rgb(0x05, 0x04, 0x13)));
        assert_eq!(c.selection_background, Some(rgb(0xa8, 0x9f, 0xa6)));
        assert_eq!(c.selection_foreground, Some(rgb(0x04, 0x04, 0x05)));
        assert_eq!(c.split_divider_color, None);
    }

    #[test]
    fn reads_split_divider_color() {
        let parsed = parse("split-divider-color = #112233\n");
        assert_eq!(parsed.colors.split_divider_color, Some(rgb(0x11, 0x22, 0x33)));
    }

    #[test]
    fn skips_comments_blank_lines_and_unknown_keys() {
        let parsed = parse("# a comment\n\nfont-size = 14\nbackground = #000000\n");
        assert!(parsed.issues.is_empty());
        assert_eq!(parsed.colors.background, Some(rgb(0, 0, 0)));
    }

    #[test]
    fn allows_spacing_quotes_and_color_names() {
        let parsed = parse("  background=\"black\"  \nforeground =   white\n");
        assert!(parsed.issues.is_empty(), "{:?}", parsed.issues);
        assert_eq!(parsed.colors.background, Some(rgb(0, 0, 0)));
        assert_eq!(parsed.colors.foreground, Some(rgb(255, 255, 255)));
    }

    #[test]
    fn a_later_line_replaces_an_earlier_line() {
        let parsed = parse("background = #111111\nbackground = #222222\npalette = 1=#aaaaaa\npalette = 1=#bbbbbb\n");
        assert_eq!(parsed.colors.background, Some(rgb(0x22, 0x22, 0x22)));
        assert_eq!(parsed.colors.palette[1], Some(rgb(0xbb, 0xbb, 0xbb)));
    }

    #[test]
    fn an_empty_value_clears_a_color() {
        let parsed = parse("background = #111111\nbackground =\n");
        assert_eq!(parsed.colors.background, None);
        assert!(parsed.issues.is_empty());
    }

    #[test]
    fn reads_the_theme_key_raw() {
        let parsed = parse("theme = light:TokyoNight Day,dark:TokyoNight\n");
        assert_eq!(parsed.theme.as_deref(), Some("light:TokyoNight Day,dark:TokyoNight"));
    }

    #[test]
    fn the_last_theme_line_wins_and_an_empty_one_clears() {
        assert_eq!(parse("theme = A\ntheme = B\n").theme.as_deref(), Some("B"));
        assert_eq!(parse("theme = A\ntheme =\n").theme, None);
    }

    #[test]
    fn palette_entries_above_15_are_skipped_without_an_issue() {
        let parsed = parse("palette = 16=#010203\npalette = 255=#040506\n");
        assert!(parsed.issues.is_empty());
        assert_eq!(parsed.colors, ThemeColors::default());
    }

    #[test]
    fn special_cell_values_are_treated_as_unset() {
        let parsed = parse("cursor-color = cell-foreground\nselection-foreground = cell-background\n");
        assert!(parsed.issues.is_empty());
        assert_eq!(parsed.colors.cursor_color, None);
        assert_eq!(parsed.colors.selection_foreground, None);
    }

    #[test]
    fn a_bad_line_gives_an_issue_with_its_line_number_and_the_rest_applies() {
        let parsed = parse("background = #000000\nforeground = notacolor\npalette = x=#010203\npalette = 3\nfoo\ncursor-color = #ffffff\n");
        assert_eq!(parsed.colors.background, Some(rgb(0, 0, 0)));
        assert_eq!(parsed.colors.foreground, None);
        assert_eq!(parsed.colors.cursor_color, Some(rgb(255, 255, 255)));
        let lines: Vec<usize> = parsed.issues.iter().map(|i| i.line).collect();
        assert_eq!(lines, vec![2, 3, 4, 5]);
        assert!(parsed.issues[0].message.contains("notacolor"));
    }

    #[test]
    fn overlay_replaces_only_the_set_colors() {
        let base = parse("background = #111111\nforeground = #eeeeee\npalette = 1=#aa0000\n").colors;
        let over = parse("background = #222222\npalette = 2=#00bb00\n").colors;
        let merged = base.overlaid_with(&over);
        assert_eq!(merged.background, Some(rgb(0x22, 0x22, 0x22)));
        assert_eq!(merged.foreground, Some(rgb(0xee, 0xee, 0xee)));
        assert_eq!(merged.palette[1], Some(rgb(0xaa, 0, 0)));
        assert_eq!(merged.palette[2], Some(rgb(0, 0xbb, 0)));
    }
}
