use i18n_embed::{
    DefaultLocalizer, LanguageLoader, Localizer,
    fluent::{FluentLanguageLoader, fluent_language_loader},
};
use rust_embed::RustEmbed;
use std::sync::LazyLock;

#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Localizations;

/// Jezyk zapasowy (en) jest ladowany zawsze; `localize()` nadpisuje go jezykiem sesji.
pub static LANGUAGE_LOADER: LazyLock<FluentLanguageLoader> = LazyLock::new(|| {
    let loader: FluentLanguageLoader = fluent_language_loader!();

    loader
        .load_fallback_language(&Localizations)
        .expect("Error while loading fallback language");
    // Dziala tylko na juz zaladowanych pakietach, wiec po kazdym ladowaniu jezyka.
    loader.set_use_isolating(false);

    loader
});

#[macro_export]
macro_rules! fl {
    ($message_id:literal) => {{
        i18n_embed_fl::fl!($crate::localize::LANGUAGE_LOADER, $message_id)
    }};

    ($message_id:literal, $($args:expr),*) => {{
        i18n_embed_fl::fl!($crate::localize::LANGUAGE_LOADER, $message_id, $($args), *)
    }};
}

pub fn localize() {
    let localizer: Box<dyn Localizer> =
        Box::new(DefaultLocalizer::new(&*LANGUAGE_LOADER, &Localizations));
    let requested = i18n_embed::DesktopLanguageRequester::requested_languages();

    if let Err(error) = localizer.select(&requested) {
        eprintln!("Error while loading language: {error}");
    }
    // `select` tworzy nowe pakiety, wiec flage trzeba ustawic ponownie.
    LANGUAGE_LOADER.set_use_isolating(false);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Idzie ta sama sciezka co `localize()`: `Localizer::select` z lista zadanych jezykow.
    fn loader_for(lang: &str) -> FluentLanguageLoader {
        let loader: FluentLanguageLoader = fluent_language_loader!();
        loader.load_fallback_language(&Localizations).unwrap();
        let localizer = DefaultLocalizer::new(&loader, &Localizations);
        localizer.select(&[lang.parse().unwrap()]).unwrap();
        loader.set_use_isolating(false);
        loader
    }

    #[test]
    fn polish_is_translated() {
        assert_eq!(loader_for("pl").get("disabled"), "Wyłączony");
    }

    #[test]
    fn unknown_language_falls_back_to_english() {
        assert_eq!(loader_for("nl").get("disabled"), "Disabled");
        assert_eq!(loader_for("ja").get("show-log"), "Show log");
    }

    #[test]
    fn arguments_are_substituted_without_isolation_marks() {
        let msg = i18n_embed_fl::fl!(loader_for("en"), "rate", value = "1.0 KiB");
        assert_eq!(msg, "1.0 KiB/s");
    }

    #[test]
    fn german_spanish_italian_french_are_translated() {
        assert_eq!(loader_for("de").get("show-log"), "Protokoll anzeigen");
        assert_eq!(loader_for("es").get("show-log"), "Mostrar registro");
        assert_eq!(loader_for("it").get("show-log"), "Mostra log");
        assert_eq!(loader_for("fr").get("show-log"), "Afficher le journal");
    }

    #[test]
    fn every_language_has_exactly_the_english_keys_and_placeholders() {
        let keys = |s: &str| {
            let mut v: Vec<_> = s
                .lines()
                .filter_map(|l| l.split_once(" = "))
                .map(|(k, val)| {
                    let mut ph: Vec<_> = val
                        .match_indices("{ $")
                        .map(|(i, _)| val[i..].split_once('}').unwrap().0.to_owned())
                        .collect();
                    ph.sort();
                    (k.to_owned(), ph)
                })
                .collect();
            v.sort();
            v
        };
        let en = keys(include_str!("../i18n/en/cosmic_applet_fortivpn.ftl"));
        for (lang, ftl) in [
            ("pl", include_str!("../i18n/pl/cosmic_applet_fortivpn.ftl")),
            ("de", include_str!("../i18n/de/cosmic_applet_fortivpn.ftl")),
            ("es", include_str!("../i18n/es/cosmic_applet_fortivpn.ftl")),
            ("it", include_str!("../i18n/it/cosmic_applet_fortivpn.ftl")),
            ("fr", include_str!("../i18n/fr/cosmic_applet_fortivpn.ftl")),
        ] {
            assert_eq!(en, keys(ftl), "language {lang} differs from en");
        }
    }
}
