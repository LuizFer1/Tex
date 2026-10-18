/// A visual design: colors for both themes, shapes, type and artwork. The `.slint` files
/// only read the `Tokens` global, so adding a design never touches them.
#[derive(Debug)]
pub struct Design {
    pub id: &'static str,
    pub light: Colors,
    pub dark: Colors,
    pub shape: Shape,
    pub typography: Typography,
    pub art: Art,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colors {
    pub action: u32,
    pub action_strong: u32,
    pub highlight: u32,
    pub paper: u32,
    pub paper_2: u32,
    pub field: u32,
    pub row_hover: u32,
    pub ink: u32,
    pub ink_2: u32,
    pub ink_3: u32,
    pub muted: u32,
    pub muted_2: u32,
    pub border: u32,
    pub border_2: u32,
    pub control_off: u32,
    pub control_off_2: u32,
    pub warn: u32,
    pub warn_text: u32,
    pub danger: u32,
    pub danger_hover: u32,
    pub danger_bg: u32,
    pub on_action: u32,
    pub on_action_muted: u32,
    pub selection: u32,
    pub type_file: u32,
    pub type_folder: u32,
    pub type_image: u32,
    pub type_app: u32,
    pub scrollbar: u32,
    pub banner: u32,
    pub banner_ink: u32,
    pub banner_ink_2: u32,
    pub banner_kicker: u32,
    pub close_hover: u32,
    pub close_hover_ink: u32,
    pub action_tint: u32,
    pub shadow: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    pub radius_control: f32,
    pub radius_card: f32,
    pub radius_tab: f32,
    pub radius_window: f32,
    pub radius_art: f32,
    pub border_control: f32,
    pub border_strong: f32,
    pub focus_width: f32,
    pub scrollbar_width: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Typography {
    pub ui_font: &'static str,
    pub mono_font: &'static str,
    pub size_search: f32,
    pub size_history_input: f32,
    pub size_title: f32,
    pub size_banner: f32,
    pub size_result: f32,
    pub size_tab: f32,
    pub size_label: f32,
    pub size_check: f32,
    pub size_body: f32,
    pub size_button: f32,
    pub size_desc: f32,
    pub size_path: f32,
    pub size_meta: f32,
    pub size_kicker: f32,
    pub size_key: f32,
    pub size_hero: f32,
    pub size_question: f32,
    pub size_badge: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Art {
    Mascot,
}

pub const DEFAULT: &str = "mascot";

pub const MASCOT: Design = Design {
    id: "mascot",
    light: Colors {
        action: 0x2E9E3F,
        action_strong: 0x1F7D30,
        highlight: 0xF9C623,
        paper: 0xFFFAEC,
        paper_2: 0xFBF1CF,
        field: 0xFFFDF5,
        row_hover: 0xFDF4D6,
        ink: 0x2A2117,
        ink_2: 0x5A4A2E,
        ink_3: 0x3F3220,
        muted: 0x7A6A48,
        muted_2: 0xA89670,
        border: 0xEAD9A8,
        border_2: 0xF0E4BD,
        control_off: 0xA08C5C,
        control_off_2: 0xD9C58D,
        warn: 0xF5A623,
        warn_text: 0x9C6400,
        danger: 0xD6452B,
        danger_hover: 0xB3321B,
        danger_bg: 0xFBE3DC,
        on_action: 0xFFFAEC,
        on_action_muted: 0xD3ECC9,
        selection: 0xFBE79A,
        type_file: 0x4A7BF7,
        type_folder: 0xF5A623,
        type_image: 0x8B45C9,
        type_app: 0x2E9E3F,
        scrollbar: 0xD9C58D,
        banner: 0xF9C623,
        banner_ink: 0x2A2117,
        banner_ink_2: 0x5A4A2E,
        banner_kicker: 0x6A5410,
        close_hover: 0xC42B1C,
        close_hover_ink: 0xFFFFFF,
        action_tint: 0xE6F2DC,
        shadow: 0x000000,
    },
    dark: Colors {
        action: 0x2E9E3F,
        action_strong: 0x5FCF6E,
        highlight: 0xF9C623,
        paper: 0x1F1A14,
        paper_2: 0x2A241A,
        field: 0x17130E,
        row_hover: 0x2A241A,
        ink: 0xF3EAD3,
        ink_2: 0xD0C3A0,
        ink_3: 0xE2D7B8,
        muted: 0xB3A37C,
        muted_2: 0x8A7A55,
        border: 0x4A3F2A,
        border_2: 0x352D1F,
        control_off: 0x8A7A55,
        control_off_2: 0x5A4C30,
        warn: 0xF5A623,
        warn_text: 0xF5A623,
        danger: 0xF26A4E,
        danger_hover: 0xD6452B,
        danger_bg: 0x4A211A,
        on_action: 0xF7FBEF,
        on_action_muted: 0xD3ECC9,
        selection: 0x6A5410,
        type_file: 0x4A7BF7,
        type_folder: 0xF5A623,
        type_image: 0x8B45C9,
        type_app: 0x2E9E3F,
        scrollbar: 0x5A4C30,
        banner: 0xF9C623,
        banner_ink: 0x2A2117,
        banner_ink_2: 0x5A4A2E,
        banner_kicker: 0x3A2E08,
        close_hover: 0xC42B1C,
        close_hover_ink: 0xFFFFFF,
        action_tint: 0x243A22,
        shadow: 0x000000,
    },
    shape: Shape {
        radius_control: 3.0,
        radius_card: 5.0,
        radius_tab: 6.0,
        radius_window: 8.0,
        radius_art: 16.0,
        border_control: 1.0,
        border_strong: 1.5,
        focus_width: 2.0,
        scrollbar_width: 10.0,
    },
    typography: Typography {
        ui_font: "Nunito",
        mono_font: "IBM Plex Mono",
        size_search: 24.0,
        size_history_input: 18.0,
        size_title: 20.0,
        size_banner: 17.0,
        size_result: 15.0,
        size_tab: 15.0,
        size_label: 14.0,
        size_check: 13.5,
        size_body: 13.0,
        size_button: 12.5,
        size_desc: 12.5,
        size_path: 12.0,
        size_meta: 11.5,
        size_kicker: 11.0,
        size_key: 10.5,
        size_hero: 26.0,
        size_question: 22.0,
        size_badge: 10.0,
    },
    art: Art::Mascot,
};

pub const DESIGNS: &[&Design] = &[&MASCOT];

pub fn design(id: &str) -> &'static Design {
    DESIGNS
        .iter()
        .copied()
        .find(|design| design.id == id)
        .unwrap_or(&MASCOT)
}

pub fn colors(design: &Design, dark: bool) -> &Colors {
    if dark { &design.dark } else { &design.light }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboarding_colors_and_sizes_exist() {
        assert_eq!(MASCOT.light.action_tint, 0xE6F2DC);
        assert_ne!(MASCOT.dark.action_tint, MASCOT.light.action_tint);
        assert_eq!(MASCOT.light.shadow, 0x000000);
        assert_eq!(MASCOT.typography.size_hero, 26.0);
        assert_eq!(MASCOT.typography.size_question, 22.0);
        assert_eq!(MASCOT.typography.size_badge, 10.0);
    }

    #[test]
    fn unknown_id_falls_back_to_the_mascot() {
        assert_eq!(design("nope").id, MASCOT.id);
        assert_eq!(design("mascot").id, "mascot");
    }

    #[test]
    fn ids_are_unique() {
        for (index, design) in DESIGNS.iter().enumerate() {
            assert!(
                DESIGNS[index + 1..]
                    .iter()
                    .all(|other| other.id != design.id)
            );
        }
    }

    #[test]
    fn dark_changes_the_paper_but_not_the_action_color() {
        let light = colors(&MASCOT, false);
        let dark = colors(&MASCOT, true);
        assert_eq!(light.paper, 0xFFFAEC);
        assert_eq!(dark.paper, 0x1F1A14);
        assert_eq!(light.action, dark.action);
    }
}
