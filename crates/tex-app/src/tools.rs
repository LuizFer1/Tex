use crate::shortcut::Shortcut;

/// The reader's own keys, which a tool can never take.
pub const READER_KEYS: [&str; 5] = ["ctrl+e", "ctrl+s", "ctrl+f", "shift+f3", "alt+z"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyProblem {
    Invalid,
    Reserved,
    Taken(String),
}

/// The shortcut each tool gets, in order: a later tool asking for a key already taken loses it.
pub fn resolve(
    requested: &[(String, Option<String>)],
) -> Vec<Result<Option<Shortcut>, KeyProblem>> {
    let mut taken: Vec<(String, String)> = Vec::new();
    requested
        .iter()
        .map(|(title, text)| {
            let Some(text) = text else { return Ok(None) };
            let shortcut = Shortcut::parse(text).ok_or(KeyProblem::Invalid)?;
            let key = shortcut.canonical();
            if READER_KEYS.contains(&key.as_str()) {
                return Err(KeyProblem::Reserved);
            }
            if let Some((_, owner)) = taken.iter().find(|(used, _)| *used == key) {
                return Err(KeyProblem::Taken(owner.clone()));
            }
            taken.push((key, title.clone()));
            Ok(Some(shortcut))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(items: &[(&str, Option<&str>)]) -> Vec<Result<Option<Shortcut>, KeyProblem>> {
        let requested: Vec<(String, Option<String>)> = items
            .iter()
            .map(|(title, key)| (title.to_string(), key.map(str::to_string)))
            .collect();
        resolve(&requested)
    }

    #[test]
    fn a_valid_shortcut_is_kept() {
        let result = ask(&[("Formatar", Some("Ctrl+Shift+F"))]);
        assert_eq!(
            result[0].clone().unwrap().unwrap().canonical(),
            "ctrl+shift+f"
        );
    }

    #[test]
    fn no_shortcut_is_fine() {
        assert_eq!(ask(&[("A", None)]), [Ok(None)]);
    }

    #[test]
    fn the_readers_keys_are_reserved() {
        for key in ["Ctrl+E", "ctrl+s", "Ctrl+F", "Shift+F3", "Alt+Z"] {
            assert_eq!(
                ask(&[("A", Some(key))]),
                [Err(KeyProblem::Reserved)],
                "{key}"
            );
        }
    }

    #[test]
    fn the_second_tool_asking_for_a_key_loses_it() {
        let result = ask(&[
            ("Formatar", Some("Ctrl+Shift+F")),
            ("Outra", Some("shift+ctrl+f")),
        ]);
        assert!(matches!(result[0], Ok(Some(_))));
        assert_eq!(result[1], Err(KeyProblem::Taken("Formatar".into())));
    }

    #[test]
    fn nonsense_is_invalid() {
        assert_eq!(ask(&[("A", Some("nada+x"))]), [Err(KeyProblem::Invalid)]);
        assert_eq!(ask(&[("A", Some("f"))]), [Err(KeyProblem::Invalid)]);
    }
}
