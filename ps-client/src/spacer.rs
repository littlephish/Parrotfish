#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpacerAlign {
    #[default]
    Left,
    Center,
    Right,
    Repeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpacerLine {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spacer {
    Text { align: SpacerAlign, text: String },
    Line(SpacerLine),
    Gap,
}

pub fn parse_spacer(name: &str, parent: u64) -> Option<Spacer> {
    if parent != 0 {
        return None;
    }
    let rest = name.strip_prefix('[')?;
    let (tag, text) = rest.split_once(']')?;
    let tag = tag.to_ascii_lowercase();
    let at = tag.find("spacer")?;
    let align = match &tag[..at] {
        "" | "l" => SpacerAlign::Left,
        "c" => SpacerAlign::Center,
        "r" => SpacerAlign::Right,
        "*" => SpacerAlign::Repeat,
        _ => return None,
    };
    let text = text.trim();
    Some(match text {
        "" => Spacer::Gap,
        "---" | "-.-" | "-.." => Spacer::Line(SpacerLine::Dashed),
        "..." => Spacer::Line(SpacerLine::Dotted),
        "___" => Spacer::Line(SpacerLine::Solid),
        _ => Spacer::Text { align, text: text.to_string() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(align: SpacerAlign, text: &str) -> Option<Spacer> {
        Some(Spacer::Text { align, text: text.to_string() })
    }

    #[test]
    fn aligned_text_spacers() {
        assert_eq!(parse_spacer("[cspacer]Games", 0), text(SpacerAlign::Center, "Games"));
        assert_eq!(parse_spacer("[lspacer]Chill", 0), text(SpacerAlign::Left, "Chill"));
        assert_eq!(parse_spacer("[spacer]Chill", 0), text(SpacerAlign::Left, "Chill"));
        assert_eq!(parse_spacer("[rspacer]est. 2019", 0), text(SpacerAlign::Right, "est. 2019"));
        assert_eq!(parse_spacer("[*spacer]-=", 0), text(SpacerAlign::Repeat, "-="));
        assert_eq!(parse_spacer("[cspacer12]Games", 0), text(SpacerAlign::Center, "Games"));
        assert_eq!(parse_spacer("[CSPACER]Games", 0), text(SpacerAlign::Center, "Games"));
        assert_eq!(parse_spacer("[cspacer0] Wide open ", 0), text(SpacerAlign::Center, "Wide open"));
        assert_eq!(parse_spacer("[cspacer]a]b", 0), text(SpacerAlign::Center, "a]b"));
    }

    #[test]
    fn line_and_gap_spacers() {
        for pattern in ["---", "-.-", "-.."] {
            assert_eq!(
                parse_spacer(&format!("[spacer0]{pattern}"), 0),
                Some(Spacer::Line(SpacerLine::Dashed)),
                "{pattern}"
            );
        }
        assert_eq!(parse_spacer("[*spacer1]...", 0), Some(Spacer::Line(SpacerLine::Dotted)));
        assert_eq!(parse_spacer("[cspacer2]___", 0), Some(Spacer::Line(SpacerLine::Solid)));
        assert_eq!(parse_spacer("[spacer3]", 0), Some(Spacer::Gap));
        assert_eq!(parse_spacer("[cspacer]   ", 0), Some(Spacer::Gap));
        assert_eq!(parse_spacer("[*spacer4]----", 0), text(SpacerAlign::Repeat, "----"));
    }

    #[test]
    fn lookalikes_stay_ordinary_channels() {
        for name in ["[cspacer", "[xspacer]A", "cspacer]A", "Lobby", "[c spacer]A", "[]A", "", "[spa cer]A", " [cspacer]A"] {
            assert_eq!(parse_spacer(name, 0), None, "{name:?}");
        }
        assert_eq!(parse_spacer("[cspacer]Games", 5), None);
    }
}
