//! Keyword matching with pinyin (F15.1): `yhgl`, `yonghu` and `用户` all find
//! "用户管理".

use pinyin::ToPinyinMulti;

/// Whether `keyword` occurs in `content`, ignoring case, or in its pinyin:
/// either the full spelling (`yonghuguanli`) or the initials (`yhgl`).
///
/// Every reading of a heteronym counts. The old app spelled out every
/// combination of readings — their product, which explodes on a long name
/// (#23) — and then searched all of them glued into one string, so a keyword
/// could match across two combinations. This walks the characters instead,
/// trying each reading only where the keyword still fits.
pub fn matches_text_or_pinyin(content: &str, keyword: &str) -> bool {
    let keyword = keyword.to_lowercase();
    if keyword.is_empty() || content.to_lowercase().contains(&keyword) {
        return true;
    }
    let readings: Vec<Vec<String>> = content.chars().map(readings_of).collect();
    matches_initials(&readings, &keyword) || matches_spelling(&readings, &keyword)
}

/// The toneless readings of a character, or the character itself (lower
/// case) when it has none. `ü` is also offered as `v` and `u`, the two ways it
/// is typed.
fn readings_of(ch: char) -> Vec<String> {
    let Some(multi) = ch.to_pinyin_multi() else {
        return vec![ch.to_lowercase().collect()];
    };
    let mut readings: Vec<String> = Vec::new();
    let mut add = |reading: String| {
        if !readings.contains(&reading) {
            readings.push(reading);
        }
    };
    for pinyin in multi {
        let plain = pinyin.plain();
        if plain.contains('ü') {
            add(plain.replace('ü', "v"));
            add(plain.replace('ü', "u"));
        }
        add(plain.to_string());
    }
    readings
}

/// The keyword, letter by letter, against consecutive characters' first
/// letters.
fn matches_initials(readings: &[Vec<String>], keyword: &str) -> bool {
    let wanted: Vec<char> = keyword.chars().collect();
    if wanted.len() > readings.len() {
        return false;
    }
    (0..=readings.len() - wanted.len()).any(|start| {
        wanted.iter().enumerate().all(|(offset, letter)| {
            readings[start + offset]
                .iter()
                .any(|reading| reading.starts_with(*letter))
        })
    })
}

/// The keyword against the spelling of consecutive characters. It may start
/// inside the first character's reading and end inside the last one's.
fn matches_spelling(readings: &[Vec<String>], keyword: &str) -> bool {
    // `failed[at][used]`: from character `at`, with `used` bytes of the
    // keyword matched, there is no way on. Each state is tried once.
    let mut failed = vec![vec![false; keyword.len() + 1]; readings.len() + 1];
    for (at, choices) in readings.iter().enumerate() {
        for reading in choices {
            for (offset, _) in reading.char_indices() {
                let tail = &reading[offset..];
                if tail.starts_with(keyword) {
                    return true;
                }
                if keyword.starts_with(tail) && continues(readings, keyword, at + 1, tail.len(), &mut failed) {
                    return true;
                }
            }
        }
    }
    false
}

fn continues(readings: &[Vec<String>], keyword: &str, at: usize, used: usize, failed: &mut [Vec<bool>]) -> bool {
    let rest = &keyword[used..];
    if rest.is_empty() {
        return true;
    }
    if at == readings.len() || failed[at][used] {
        return false;
    }
    for reading in &readings[at] {
        if reading.starts_with(rest) {
            return true;
        }
        if rest.starts_with(reading.as_str()) && continues(readings, keyword, at + 1, used + reading.len(), failed) {
            return true;
        }
    }
    failed[at][used] = true;
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_ignores_case() {
        assert!(matches_text_or_pinyin("User API", "api"));
        assert!(matches_text_or_pinyin("User API", "USER"));
        assert!(!matches_text_or_pinyin("User API", "order"));
        assert!(matches_text_or_pinyin("anything", ""));
    }

    #[test]
    fn full_spelling_and_initials() {
        assert!(matches_text_or_pinyin("用户管理", "yonghu"));
        assert!(matches_text_or_pinyin("用户管理", "yonghuguanli"));
        assert!(matches_text_or_pinyin("用户管理", "huguan"));
        assert!(matches_text_or_pinyin("用户管理", "yhgl"));
        assert!(matches_text_or_pinyin("用户管理", "hg"));
        assert!(!matches_text_or_pinyin("用户管理", "guanyong"));
        assert!(!matches_text_or_pinyin("用户管理", "yg"));
    }

    #[test]
    fn a_match_may_start_and_end_inside_a_syllable() {
        assert!(matches_text_or_pinyin("用户管理", "nghug"));
        assert!(matches_text_or_pinyin("用户", "ongh"));
    }

    #[test]
    fn mixed_text_spells_through() {
        assert!(matches_text_or_pinyin("获取token", "huoqutoken"));
        assert!(matches_text_or_pinyin("获取token", "hqtoken"));
        assert!(matches_text_or_pinyin("v1用户", "v1yh"));
    }

    #[test]
    fn every_reading_of_a_heteronym_counts() {
        // 行: xing / hang
        assert!(matches_text_or_pinyin("银行", "yinhang"));
        assert!(matches_text_or_pinyin("行为", "xingwei"));
        assert!(matches_text_or_pinyin("银行", "yh"));
        assert!(matches_text_or_pinyin("银行", "yx"));
    }

    #[test]
    fn u_umlaut_is_typed_as_v_or_u() {
        assert!(matches_text_or_pinyin("绿色", "lvse"));
        assert!(matches_text_or_pinyin("绿色", "luse"));
    }

    #[test]
    fn readings_do_not_join_across_combinations() {
        // The old matcher glued every combination into one string, so the
        // end of one and the start of the next could match together.
        assert!(!matches_text_or_pinyin("行行", "hangxinghang"));
    }

    #[test]
    fn a_long_name_stays_cheap() {
        // 30 heteronyms: the old approach would spell out 2^30 combinations.
        let name = "行".repeat(30);
        assert!(!matches_text_or_pinyin(&name, "hangxingz"));
        assert!(matches_text_or_pinyin(&name, "hangxinghang"));
    }
}
