use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use log::error;
use serde::de::Error as SerdeError;
use serde::{self, Deserialize, Deserializer};
use unicode_width::UnicodeWidthChar;

use alacritty_config_derive::ConfigDeserialize;
use alacritty_terminal::config::{Percentage, Program, LOG_TARGET_CONFIG};
use alacritty_terminal::term::search::RegexSearch;

use crate::config::bell::BellConfig;
use crate::config::bindings::{
    self, Action, Binding, BindingMode, Key, KeyBinding, ModsWrapper, MouseBinding,
};
use crate::config::color::Colors;
use crate::config::debug::Debug;
use crate::config::font::Font;
use crate::config::mouse::Mouse;
use crate::config::window::WindowConfig;

/// Regex used for the default URL hint.
#[rustfmt::skip]
const URL_REGEX: &str = "(mailto:|gemini:|gopher:|https:|http:|news:|file:|git:|ssh:|ftp:)\
                         [^\u{0000}-\u{001F}\u{007F}-\u{009F}<>\" {-}\\^⟨⟩`]+";

#[derive(ConfigDeserialize, Debug, PartialEq)]
pub struct UIConfig {
    /// Font configuration.
    pub font: Font,

    /// Window configuration.
    pub window: WindowConfig,

    pub grep_after: usize,

    pub mouse: Mouse,

    /// Debug options.
    pub debug: Debug,

    /// Send escape sequences using the alt key.
    pub alt_send_esc: bool,

    /// Live config reload.
    pub live_config_reload: bool,

    /// Bell configuration.
    pub bell: BellConfig,

    /// RGB values for colors.
    pub colors: Colors,

    /// Should draw bold text with brighter colors instead of bold font.
    pub draw_bold_text_with_bright_colors: bool,

    /// Path where config was loaded from.
    #[config(skip)]
    pub config_paths: Vec<PathBuf>,

    /// Regex hints for interacting with terminal content.
    pub hints: Hints,

    /// Keybindings.
    key_bindings: KeyBindings,

    /// Bindings for the mouse.
    mouse_bindings: MouseBindings,

    /// Background opacity from 0.0 to 1.0.
    background_opacity: Percentage,
}

impl Clone for UIConfig {
    fn clone(&self) -> Self {
        Self {
            font: self.font.clone(),
            window: self.window.clone(),
            grep_after: self.grep_after.clone(),
            mouse: self.mouse.clone(),
            debug: self.debug.clone(),
            alt_send_esc: self.alt_send_esc,
            live_config_reload: self.live_config_reload,
            bell: self.bell.clone(),
            colors: self.colors.clone(),
            draw_bold_text_with_bright_colors: self.draw_bold_text_with_bright_colors,
            config_paths: self.config_paths.clone(),
            hints: Hints::default(),
            key_bindings: self.key_bindings.clone(),
            mouse_bindings: self.mouse_bindings.clone(),
            background_opacity: self.background_opacity.clone()
        }
    }
}

impl Default for UIConfig {
    fn default() -> Self {
        Self {
            alt_send_esc: true,
            live_config_reload: true,
            font: Default::default(),
            window: Default::default(),
            grep_after: Default::default(),
            mouse: Default::default(),
            debug: Default::default(),
            config_paths: Default::default(),
            key_bindings: Default::default(),
            mouse_bindings: Default::default(),
            background_opacity: Default::default(),
            bell: Default::default(),
            colors: Default::default(),
            draw_bold_text_with_bright_colors: Default::default(),
            hints: Default::default(),
        }
    }
}

impl UIConfig {
    /// Generate key bindings for all keyboard hints.
    pub fn generate_hint_bindings(&mut self) {
        for hint in &self.hints.enabled {
            let binding = match hint.binding {
                Some(binding) => binding,
                None => continue,
            };

            let binding = KeyBinding {
                trigger: binding.key,
                mods: binding.mods.0,
                mode: BindingMode::empty(),
                notmode: BindingMode::empty(),
                action: Action::Hint(hint.clone()),
            };

            self.key_bindings.0.push(binding);
        }
    }

    #[inline]
    pub fn background_opacity(&self) -> f32 {
        self.background_opacity.as_f32()
    }

    #[inline]
    pub fn key_bindings(&self) -> &[KeyBinding] {
        &self.key_bindings.0.as_slice()
    }

    #[inline]
    pub fn mouse_bindings(&self) -> &[MouseBinding] {
        self.mouse_bindings.0.as_slice()
    }
}

#[derive(Debug, Clone, PartialEq)]
struct KeyBindings(Vec<KeyBinding>);

impl Default for KeyBindings {
    fn default() -> Self {
        Self(bindings::default_key_bindings())
    }
}

impl<'de> Deserialize<'de> for KeyBindings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self(deserialize_bindings(deserializer, Self::default().0)?))
    }
}

#[derive(Debug, Clone, PartialEq)]
struct MouseBindings(Vec<MouseBinding>);

impl Default for MouseBindings {
    fn default() -> Self {
        Self(bindings::default_mouse_bindings())
    }
}

impl<'de> Deserialize<'de> for MouseBindings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self(deserialize_bindings(deserializer, Self::default().0)?))
    }
}

fn deserialize_bindings<'a, D, T>(
    deserializer: D,
    mut default: Vec<Binding<T>>,
) -> Result<Vec<Binding<T>>, D::Error>
where
    D: Deserializer<'a>,
    T: Copy + Eq,
    Binding<T>: Deserialize<'a>,
{
    let values = Vec::<serde_yaml::Value>::deserialize(deserializer)?;

    // Skip all invalid values.
    let mut bindings = Vec::with_capacity(values.len());
    for value in values {
        match Binding::<T>::deserialize(value) {
            Ok(binding) => bindings.push(binding),
            Err(err) => {
                error!(target: LOG_TARGET_CONFIG, "Config error: {}; ignoring binding", err);
            },
        }
    }

    // Remove matching default bindings.
    for binding in bindings.iter() {
        default.retain(|b| !b.triggers_match(binding));
    }

    bindings.extend(default);

    Ok(bindings)
}

/// A delta for a point in a 2 dimensional plane.
#[derive(ConfigDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Delta<T: Default> {
    /// Horizontal change.
    pub x: T,
    /// Vertical change.
    pub y: T,
}

/// Regex terminal hints.
#[derive(ConfigDeserialize, Debug, PartialEq, Eq)]
pub struct Hints {
    /// Characters for the hint labels.
    alphabet: HintsAlphabet,

    /// All configured terminal hints.
    pub enabled: Vec<Hint>,
}

impl Default for Hints {
    fn default() -> Self {
        // Add URL hint by default when no other hint is present.
        let pattern = LazyRegexVariant::Pattern(String::from(URL_REGEX));
        let regex = LazyRegex(Rc::new(RefCell::new(pattern)));

        #[cfg(not(any(target_os = "macos", windows)))]
        let action = HintAction::Command(Program::Just(String::from("xdg-open")));
        #[cfg(target_os = "macos")]
        let action = HintAction::Command(Program::Just(String::from("open")));
        #[cfg(windows)]
        let action = HintAction::Command(Program::WithArgs {
            program: String::from("cmd"),
            args: vec!["/c".to_string(), "start".to_string(), "".to_string()],
        });

        Self {
            enabled: vec![Hint {
                regex,
                action,
                post_processing: true,
                mouse: Some(HintMouse { enabled: true, mods: Default::default() }),
                binding: Default::default(),
            }],
            alphabet: Default::default(),
        }
    }
}

impl Hints {
    /// Characters for the hint labels.
    pub fn alphabet(&self) -> &str {
        &self.alphabet.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HintsAlphabet(String);

impl Default for HintsAlphabet {
    fn default() -> Self {
        Self(String::from("jfkdls;ahgurieowpq"))
    }
}

impl<'de> Deserialize<'de> for HintsAlphabet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;

        let mut character_count = 0;
        for character in value.chars() {
            if character.width() != Some(1) {
                return Err(D::Error::custom("characters must be of width 1"));
            }
            character_count += 1;
        }

        if character_count < 2 {
            return Err(D::Error::custom("must include at last 2 characters"));
        }

        Ok(Self(value))
    }
}

/// Built-in actions for hint mode.
#[derive(ConfigDeserialize, Clone, Debug, PartialEq, Eq)]
pub enum HintInternalAction {
    /// Copy the text to the clipboard.
    Copy,
    /// Write the text to the PTY/search.
    Paste,
    /// Select the text matching the hint.
    Select,
    /// Move the vi mode cursor to the beginning of the hint.
    MoveViModeCursor,
}

/// Actions for hint bindings.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum HintAction {
    /// Built-in hint action.
    #[serde(rename = "action")]
    Action(HintInternalAction),

    /// Command the text will be piped to.
    #[serde(rename = "command")]
    Command(Program),
}

/// Hint configuration.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    /// Regex for finding matches.
    pub regex: LazyRegex,

    /// Action executed when this hint is triggered.
    #[serde(flatten)]
    pub action: HintAction,

    /// Hint text post processing.
    #[serde(default)]
    pub post_processing: bool,

    /// Hint mouse highlighting.
    pub mouse: Option<HintMouse>,

    /// Binding required to search for this hint.
    binding: Option<HintBinding>,
}

/// Binding for triggering a keyboard hint.
#[derive(Deserialize, Copy, Clone, Debug, PartialEq, Eq)]
pub struct HintBinding {
    pub key: Key,
    #[serde(default)]
    pub mods: ModsWrapper,
}

/// Hint mouse highlighting.
#[derive(ConfigDeserialize, Default, Copy, Clone, Debug, PartialEq, Eq)]
pub struct HintMouse {
    /// Hint mouse highlighting availability.
    pub enabled: bool,

    /// Required mouse modifiers for hint highlighting.
    pub mods: ModsWrapper,
}

/// Lazy regex with interior mutability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LazyRegex(Rc<RefCell<LazyRegexVariant>>);

impl LazyRegex {
    /// Execute a function with the compiled regex DFAs as parameter.
    pub fn with_compiled<T, F>(&self, mut f: F) -> T
    where
        F: FnMut(&RegexSearch) -> T,
    {
        f(self.0.borrow_mut().compiled())
    }
}

impl<'de> Deserialize<'de> for LazyRegex {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let regex = LazyRegexVariant::Pattern(String::deserialize(deserializer)?);
        Ok(Self(Rc::new(RefCell::new(regex))))
    }
}

/// Regex which is compiled on demand, to avoid expensive computations at startup.
#[derive(Clone, Debug)]
pub enum LazyRegexVariant {
    Compiled(Box<RegexSearch>),
    Pattern(String),
}

impl LazyRegexVariant {
    /// Get a reference to the compiled regex.
    ///
    /// If the regex is not already compiled, this will compile the DFAs and store them for future
    /// access.
    fn compiled(&mut self) -> &RegexSearch {
        // Check if the regex has already been compiled.
        let regex = match self {
            Self::Compiled(regex_search) => return regex_search,
            Self::Pattern(regex) => regex,
        };

        // Compile the regex.
        let regex_search = match RegexSearch::new(&regex) {
            Ok(regex_search) => regex_search,
            Err(error) => {
                error!("hint regex is invalid: {}", error);
                RegexSearch::new("").unwrap()
            },
        };
        *self = Self::Compiled(Box::new(regex_search));

        // Return a reference to the compiled DFAs.
        match self {
            Self::Compiled(dfas) => dfas,
            Self::Pattern(_) => unreachable!(),
        }
    }
}

impl PartialEq for LazyRegexVariant {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Pattern(regex), Self::Pattern(other_regex)) => regex == other_regex,
            _ => false,
        }
    }
}
impl Eq for LazyRegexVariant {}
