// SPDX-License-Identifier: MPL-2.0

use std::sync::LazyLock;

use i18n_embed::{
    DefaultLocalizer, LanguageLoader, Localizer,
    fluent::{FluentLanguageLoader, fluent_language_loader},
    unic_langid::LanguageIdentifier,
};
use rust_embed::RustEmbed;

pub fn init(requested_languages: &[LanguageIdentifier]) {
    let localizer = DefaultLocalizer::new(&*LANGUAGE_LOADER, &Localizations);
    if let Err(error) = localizer.select(requested_languages) {
        eprintln!("error while loading localizations: {error}");
    }
}

#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Localizations;

pub static LANGUAGE_LOADER: LazyLock<FluentLanguageLoader> = LazyLock::new(|| {
    let loader: FluentLanguageLoader = fluent_language_loader!();
    loader
        .load_fallback_language(&Localizations)
        .expect("failed to load fallback language");
    loader
});

#[macro_export]
macro_rules! fl {
    ($message_id:literal) => {{
        $crate::i18n::LANGUAGE_LOADER.get($message_id)
    }};
    ($message_id:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        let mut args = std::collections::HashMap::new();
        $(args.insert(stringify!($name), ($value).into());)+
        $crate::i18n::LANGUAGE_LOADER.get_args_concrete($message_id, args)
    }};
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_ui_message_is_available_and_formats() {
        for id in [
            "app-title",
            "popup-title",
            "empty-list",
            "loading",
            "kill",
            "killing",
        ] {
            assert_ne!(
                super::LANGUAGE_LOADER.get(id),
                format!("No localization for id: \"{id}\"")
            );
        }
        assert!(crate::fl!("pid", pid = 42_u32).contains("42"));
        assert!(crate::fl!("process-count", count = 2_usize).contains('2'));
        assert!(crate::fl!("kill-failed", name = "Spotify", error = "denied").contains("Spotify"));
        assert!(crate::fl!("scan-failed", error = "denied").contains("denied"));
    }
}
