//! Genre tags. Every song carries exactly one genre; the sidebar's genre list is built from them.

/// ID3v1 genre table (indices 0-79), used when a tag stores a number such as "(17)".
const ID3: [&str; 80] = [
    "Blues",
    "Classic Rock",
    "Country",
    "Dance",
    "Disco",
    "Funk",
    "Grunge",
    "Hip-Hop",
    "Jazz",
    "Metal",
    "New Age",
    "Oldies",
    "Other",
    "Pop",
    "R&B",
    "Rap",
    "Reggae",
    "Rock",
    "Techno",
    "Industrial",
    "Alternative",
    "Ska",
    "Death Metal",
    "Pranks",
    "Soundtrack",
    "Euro-Techno",
    "Ambient",
    "Trip-Hop",
    "Vocal",
    "Jazz+Funk",
    "Fusion",
    "Trance",
    "Classical",
    "Instrumental",
    "Acid",
    "House",
    "Game",
    "Sound Clip",
    "Gospel",
    "Noise",
    "Alternative Rock",
    "Bass",
    "Soul",
    "Punk",
    "Space",
    "Meditative",
    "Instrumental Pop",
    "Instrumental Rock",
    "Ethnic",
    "Gothic",
    "Darkwave",
    "Techno-Industrial",
    "Electronic",
    "Pop-Folk",
    "Eurodance",
    "Dream",
    "Southern Rock",
    "Comedy",
    "Cult",
    "Gangsta",
    "Top 40",
    "Christian Rap",
    "Pop/Funk",
    "Jungle",
    "Native American",
    "Cabaret",
    "New Wave",
    "Psychedelic",
    "Rave",
    "Showtunes",
    "Trailer",
    "Lo-Fi",
    "Tribal",
    "Acid Punk",
    "Acid Jazz",
    "Polka",
    "Retro",
    "Musical",
    "Rock & Roll",
    "Hard Rock",
];

pub const UNSORTED: &str = "Unsorted";

/// Turns whatever a tag or a person typed into the display form of a genre.
/// Numeric ID3 genres become names, only the first of several genres is kept,
/// all-lowercase input is title-cased, and empty input becomes "Unsorted".
pub fn normalize(raw: &str) -> String {
    let mut g = raw.trim().to_string();
    if let Some(rest) = g.strip_prefix('(') {
        if let Some(close) = rest.find(')') {
            let (num, tail) = rest.split_at(close);
            let tail = tail[1..].trim();
            g = if !tail.is_empty() {
                tail.to_string()
            } else {
                num.parse::<usize>()
                    .ok()
                    .and_then(|i| ID3.get(i))
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            };
        }
    } else if !g.is_empty() && g.chars().all(|c| c.is_ascii_digit()) {
        g = g
            .parse::<usize>()
            .ok()
            .and_then(|i| ID3.get(i))
            .map(|s| s.to_string())
            .unwrap_or_default();
    }
    let g = g
        .split([';', '/', '\0'])
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if g.is_empty() {
        return UNSORTED.to_string();
    }
    if g.chars().any(|c| c.is_alphabetic()) && g == g.to_lowercase() {
        return title_case(&g);
    }
    g
}

/// Key used to merge genres that differ only by case.
pub fn key(raw: &str) -> String {
    normalize(raw).to_lowercase()
}

fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut start = true;
    for c in s.chars() {
        if start && c.is_alphabetic() {
            out.extend(c.to_uppercase());
            start = false;
        } else {
            out.push(c);
            if !c.is_alphanumeric() {
                start = true;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_tags() {
        assert_eq!(normalize("(17)"), "Rock");
        assert_eq!(normalize("17"), "Rock");
        assert_eq!(normalize("(17)Rock"), "Rock");
        assert_eq!(normalize("hip hop"), "Hip Hop");
        assert_eq!(normalize("Indie Folk; Folk"), "Indie Folk");
        assert_eq!(normalize("  "), "Unsorted");
        assert_eq!(normalize("R&B"), "R&B");
        assert_eq!(key("rock"), key("Rock"));
    }
}
