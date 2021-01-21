use dialoguer::{console::style, theme::ColorfulTheme};
use indicatif::ProgressStyle;
use lazy_static::{initialize, lazy_static};

#[cfg(target_os = "windows")]
lazy_static! {
    pub static ref THEME: ColorfulTheme = ColorfulTheme {
        prompt_suffix: style(">".to_string()).for_stderr().black().bright(),
        active_item_prefix: style(">".to_string()).for_stderr().green(),
        picked_item_prefix: style(">".to_string()).for_stderr().green(),
        success_prefix: style("√".to_string()).for_stderr().green(),
        error_prefix: style("×".to_string()).for_stderr().red(),
        ..ColorfulTheme::default()
    };
    pub static ref SPINNER_STYLE: ProgressStyle =
        ProgressStyle::default_spinner().tick_chars("▁▃▅▇█▇▅▃▁√");
}

#[cfg(not(target_os = "windows"))]
lazy_static! {
    pub static ref THEME: ColorfulTheme = ColorfulTheme::default();
    pub static ref SPINNER_STYLE: ProgressStyle = ProgressStyle::default_spinner();
}

pub fn init() {
    #[cfg(target_os = "windows")]
    {
        use win32console::console::WinConsole;
        use win32console::structs::console_font_info_ex::ConsoleFontInfoEx;
        use win32console::structs::coord::Coord;
        WinConsole::set_input_code(65001).expect("unable to set console encoding");
        WinConsole::set_output_code(65001).expect("unable to set console encoding");
        let font_name_vec: Vec<u16> = "SimHei".encode_utf16().collect();
        let mut font_name = [0; 32];
        font_name[..font_name_vec.len()].clone_from_slice(&font_name_vec);
        let font = ConsoleFontInfoEx {
            size: std::mem::size_of::<ConsoleFontInfoEx>() as u32,
            font_index: 0,
            font_size: Coord { x: 14, y: 27 },
            font_family: 54,
            font_weight: 400,
            face_name: font_name,
        };
        WinConsole::output()
            .set_font_ex(font, false)
            .expect("unable to set console font");
    }
    initialize(&THEME);
    initialize(&SPINNER_STYLE);
}
