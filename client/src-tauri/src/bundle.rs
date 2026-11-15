use crate::model::Recipe;
use std::collections::BTreeMap;

include!(concat!(env!("OUT_DIR"), "/applications.rs"));

/// Companion scripts travel with their application, never with VM boot.
pub fn files(application: &Recipe) -> BTreeMap<String, String> {
    if !application.builtin {
        return BTreeMap::new();
    }
    let prefix = format!("{}/", application.id);
    FILES
        .iter()
        .filter_map(|(name, content)| {
            name.strip_prefix(&prefix)
                .filter(|relative| *relative != "manifest.json")
                .map(|relative| (relative.to_owned(), (*content).to_owned()))
        })
        .collect()
}
