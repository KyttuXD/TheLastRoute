//! Void hunters' own powers (game/hunters/gimmicks/void_*_gimmick.gd), one file per hunter.

pub mod void_black;
pub mod void_blue;
pub mod void_brown;
pub mod void_cyan;
pub mod void_gray;
pub mod void_green;
pub mod void_orange;
pub mod void_pink;
pub mod void_purple;
pub mod void_red;
pub mod void_white;
pub mod void_yellow;

/// The dossier text of a void hunter: the upper-case `dossier` of data/hunters/<id>.json (None for any other id).
pub fn dossier(id: &str) -> Option<&'static str> {
    static TEXTS: std::sync::OnceLock<Vec<(&'static str, String)>> = std::sync::OnceLock::new();
    macro_rules! row {
        ($id:literal) => {
            ($id, include_str!(concat!("../../../data/hunters/", $id, ".json")))
        };
    }
    let rows = TEXTS.get_or_init(|| {
        [
            row!("void_black"), row!("void_blue"), row!("void_brown"), row!("void_cyan"), row!("void_gray"), row!("void_green"),
            row!("void_orange"), row!("void_pink"), row!("void_purple"), row!("void_red"), row!("void_white"), row!("void_yellow"),
        ]
        .iter()
        .filter_map(|(id, text)| {
            let v: serde_json::Value = serde_json::from_str(text).ok()?;
            Some((*id, v.get("dossier")?.as_str()?.to_string()))
        })
        .collect()
    });
    rows.iter().find(|(k, _)| *k == id).map(|(_, t)| t.as_str())
}

#[cfg(test)]
mod dossier_tests {
    use super::dossier;

    /// Every void hunter ships an upper-case dossier text from its data file; other ids have none.
    #[test]
    fn every_void_hunter_has_an_upper_case_dossier_text() {
        for id in ["void_black", "void_blue", "void_brown", "void_cyan", "void_gray", "void_green", "void_orange", "void_pink", "void_purple", "void_red", "void_white", "void_yellow"] {
            let t = dossier(id).unwrap_or_else(|| panic!("{id}: no dossier"));
            assert!(t.len() > 40, "{id}: too short");
            assert_eq!(t, t.to_uppercase(), "{id}: not upper case");
        }
        assert!(dossier("dovelle").is_none());
    }
}
