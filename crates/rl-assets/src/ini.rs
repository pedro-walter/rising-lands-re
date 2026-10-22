//! Parser for the game's rule files (`RISING.INI`, `MONSTRE.INI`, `EVENTS.INI`).
//!
//! They are Latin-1 text with `[SECTION]` headers, `KEY=v1,v2,...` lines and
//! `;` comments. Keys may repeat inside a section (e.g. `COUT`), so entries
//! are kept in file order.

#[derive(Debug, Default, Clone)]
pub struct Section {
    pub name: String,
    pub entries: Vec<(String, String)>,
}

impl Section {
    /// First value for `key` (case-insensitive).
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }

    /// First value for `key` parsed as a comma-separated list of integers.
    pub fn ints(&self, key: &str) -> Option<Vec<i32>> {
        let v = self.get(key)?;
        v.split(',').map(|s| s.trim().parse().ok()).collect()
    }

    pub fn int(&self, key: &str) -> Option<i32> {
        self.ints(key)?.first().copied()
    }
}

#[derive(Debug, Default, Clone)]
pub struct Ini {
    pub sections: Vec<Section>,
}

impl Ini {
    pub fn parse(bytes: &[u8]) -> Ini {
        // Latin-1 maps bytes 1:1 onto the first 256 code points.
        let text: String = bytes.iter().map(|&b| b as char).collect();
        let mut ini = Ini::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.split(']').next()) {
                ini.sections.push(Section { name: name.trim().to_string(), entries: Vec::new() });
            } else if let (Some((k, v)), Some(sec)) = (line.split_once('='), ini.sections.last_mut()) {
                // Trailing `;` comments are allowed after values.
                let v = v.split(';').next().unwrap_or("").trim();
                sec.entries.push((k.trim().to_string(), v.to_string()));
            }
        }
        ini
    }

    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_and_ints() {
        let ini = Ini::parse(b";c\n[SOLDAT]\nDEGAT=20, 10,5\nRAYON=7 ; sight\nNAME=SOLDADO\n");
        let s = ini.section("soldat").unwrap();
        assert_eq!(s.ints("DEGAT"), Some(vec![20, 10, 5]));
        assert_eq!(s.int("RAYON"), Some(7));
        assert_eq!(s.get("NAME"), Some("SOLDADO"));
    }
}
