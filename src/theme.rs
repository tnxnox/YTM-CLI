use comfy_table::{Attribute, Color, Table, modifiers, presets};
use dialoguer::console::Style as DialoguerStyle;
use dialoguer::theme::ColorfulTheme;
use owo_colors::{OwoColorize, Rgb, Style};

// Theme color constants using RGB for owo-colors
pub const COLOR_PRIMARY: (u8, u8, u8) = (147, 112, 219); // Medium Purple
pub const COLOR_SECONDARY: (u8, u8, u8) = (72, 61, 139); // Dark Slate Blue
pub const COLOR_ACCENT: (u8, u8, u8) = (186, 85, 211); // Medium Orchid
pub const COLOR_DIM: (u8, u8, u8) = (119, 136, 153); // Light Slate Gray

pub fn print_banner() {
    let purple = Rgb(COLOR_PRIMARY.0, COLOR_PRIMARY.1, COLOR_PRIMARY.2);
    let blue = Rgb(COLOR_SECONDARY.0, COLOR_SECONDARY.1, COLOR_SECONDARY.2);
    let accent = Rgb(COLOR_ACCENT.0, COLOR_ACCENT.1, COLOR_ACCENT.2);

    println!();
    println!(
        "{}",
        "  ┌────────────────────────────────────────────────────────┐".color(blue)
    );
    println!(
        "  {}                {}                {}",
        "│".color(blue),
        "🎧  YOUTUBE MUSIC CLI  🎧".color(purple).bold(),
        "│".color(blue)
    );
    println!(
        "{}",
        "  └────────────────────────────────────────────────────────┘".color(blue)
    );

    let ascii_art = r#"
      __   __ _____ __  __    ____ _     ___ 
      \ \ / /|_   _|  \/  |  / ___| |   |_ _|
       \ V /   | | | |\/| | | |   | |    | | 
        | |    | | | |  | | | |___| |___ | | 
        |_|    |_| |_|  |_|  \____|_____|___|
    "#;

    for line in ascii_art.lines() {
        if !line.trim().is_empty() {
            println!("  {}", line.color(accent).bold());
        }
    }
    println!();
}

pub fn style_primary_style() -> Style {
    Style::new()
        .color(Rgb(COLOR_PRIMARY.0, COLOR_PRIMARY.1, COLOR_PRIMARY.2))
        .bold()
}

pub fn style_secondary_style() -> Style {
    Style::new().color(Rgb(COLOR_SECONDARY.0, COLOR_SECONDARY.1, COLOR_SECONDARY.2))
}

pub fn style_accent_style() -> Style {
    Style::new()
        .color(Rgb(COLOR_ACCENT.0, COLOR_ACCENT.1, COLOR_ACCENT.2))
        .bold()
}

pub fn style_dim_style() -> Style {
    Style::new().color(Rgb(COLOR_DIM.0, COLOR_DIM.1, COLOR_DIM.2))
}

pub fn style_error_style() -> Style {
    Style::new().red().bold()
}

/// Helper to style strings with the primary theme color without allocations
pub fn style_primary(s: &str) -> impl std::fmt::Display + '_ {
    style_primary_style().style(s)
}

/// Helper to style strings with the secondary theme color without allocations
pub fn style_secondary(s: &str) -> impl std::fmt::Display + '_ {
    style_secondary_style().style(s)
}

/// Helper to style strings with the accent color without allocations
pub fn style_accent(s: &str) -> impl std::fmt::Display + '_ {
    style_accent_style().style(s)
}

/// Helper to style dim text without allocations
pub fn style_dim(s: &str) -> impl std::fmt::Display + '_ {
    style_dim_style().style(s)
}

/// Helper to style error messages without allocations
pub fn style_error(s: &str) -> impl std::fmt::Display + '_ {
    style_error_style().style(s)
}

/// Returns a customized dialoguer ColorfulTheme aligned with our deep blue / purple aesthetic
pub fn get_dialoguer_theme() -> ColorfulTheme {
    let mut theme = ColorfulTheme::default();
    theme.defaults_style = DialoguerStyle::new().cyan();
    theme.prompt_style = DialoguerStyle::new().bold().color256(141); // Orchid/Purple
    theme.prompt_prefix = DialoguerStyle::new()
        .color256(61)
        .bold()
        .apply_to("?".to_string()); // Dark Slate Blue / Indigo
    theme.prompt_suffix = DialoguerStyle::new()
        .color256(244)
        .apply_to("»".to_string()); // Dim gray
    theme.success_prefix = DialoguerStyle::new()
        .bold()
        .green()
        .apply_to("✔".to_string());
    theme.success_suffix = DialoguerStyle::new()
        .color256(244)
        .apply_to("·".to_string());
    theme.error_style = DialoguerStyle::new().red().bold();
    theme.error_prefix = DialoguerStyle::new().red().bold().apply_to("✘".to_string());
    theme.active_item_style = DialoguerStyle::new().bold().color256(141); // Orchid/Purple highlight
    theme.inactive_item_style = DialoguerStyle::new().color256(61); // Slate Blue
    theme.active_item_prefix = DialoguerStyle::new()
        .color256(186)
        .bold()
        .apply_to(">".to_string()); // Accent pointer
    theme.inactive_item_prefix = DialoguerStyle::new()
        .color256(240)
        .apply_to(" ".to_string());
    theme.checked_item_prefix = DialoguerStyle::new()
        .color256(186)
        .bold()
        .apply_to("[x]".to_string());
    theme.unchecked_item_prefix = DialoguerStyle::new()
        .color256(240)
        .apply_to("[ ]".to_string());
    theme.hint_style = DialoguerStyle::new().color256(244);
    theme.values_style = DialoguerStyle::new().cyan();
    theme
}

/// Creates a new comfy-table with our custom styling preset
pub fn create_styled_table() -> Table {
    let mut table = Table::new();

    // Use nice rounded/UTF-8 box borders
    table.load_preset(presets::UTF8_FULL);
    table.apply_modifier(modifiers::UTF8_ROUND_CORNERS);

    // Apply styling rules: Purple/blue header with dim borders
    table.set_content_arrangement(comfy_table::ContentArrangement::Dynamic);

    table
}

/// Stylizes header cell for comfy-table
pub fn style_header_cell(s: &str) -> comfy_table::Cell {
    comfy_table::Cell::new(s)
        .fg(Color::Rgb {
            r: COLOR_PRIMARY.0,
            g: COLOR_PRIMARY.1,
            b: COLOR_PRIMARY.2,
        })
        .add_attribute(Attribute::Bold)
}

/// Stylizes data cell for comfy-table
pub fn style_data_cell(s: &str, is_accent: bool) -> comfy_table::Cell {
    let cell = comfy_table::Cell::new(s);
    if is_accent {
        cell.fg(Color::Rgb {
            r: COLOR_ACCENT.0,
            g: COLOR_ACCENT.1,
            b: COLOR_ACCENT.2,
        })
    } else {
        cell.fg(Color::Rgb {
            r: COLOR_DIM.0,
            g: COLOR_DIM.1,
            b: COLOR_DIM.2,
        })
    }
}
