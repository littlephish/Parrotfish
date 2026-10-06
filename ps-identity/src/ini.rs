#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub entries: Vec<(String, String)>,
}

pub fn parse(text: &str) -> Result<Vec<Section>, String> {
    let mut sections: Vec<Section> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim()
                .to_string();
            sections.push(Section { name, entries: Vec::new() });
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim().to_string();
            let v = unquote(v.trim());
            match sections.last_mut() {
                Some(s) => s.entries.push((k, v)),
                None => sections
                    .push(Section { name: String::new(), entries: vec![(k, v)] }),
            }
        }
    }
    Ok(sections)
}

fn unquote(v: &str) -> String {
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        v[1..v.len() - 1].to_string()
    } else {
        v.to_string()
    }
}

pub fn get<'a>(sections: &'a [Section], section: &str, key: &str) -> Option<&'a str> {
    sections
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(section))
        .and_then(|s| s.entries.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)))
        .map(|(_, v)| v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_identity_file() {
        let text = "[Identity]\nid=Default\nidentity=\"213Vabc/def==\"\nnickname=LittlePhish\nphonetic_nickname=\n";
        let sections = parse(text).unwrap();
        assert_eq!(get(&sections, "identity", "identity").unwrap(), "213Vabc/def==");
        assert_eq!(get(&sections, "Identity", "nickname").unwrap(), "LittlePhish");
        assert_eq!(get(&sections, "identity", "phonetic_nickname").unwrap(), "");
    }
}
