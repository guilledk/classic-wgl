//! Charset groups, ported from `make-font-atlas.mjs`.

use std::collections::HashSet;

fn range(a: u32, b: u32) -> Vec<char> {
    (a..=b).filter_map(char::from_u32).collect()
}

fn chars_of(s: &str) -> Vec<char> {
    s.chars().collect()
}

/// The full shipped charset (the mjs `resolveCharset('all')`): every group,
/// concatenated and de-duplicated in order.
pub fn full_charset() -> Vec<char> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for ch in all_groups() {
        if seen.insert(ch) {
            out.push(ch);
        }
    }
    out
}

fn all_groups() -> Vec<char> {
    let mut v = Vec::new();
    v.extend(range(0x0020, 0x007e)); // ascii
    v.extend(range(0x00a0, 0x00ff)); // latin1
    v.extend(chars_of(
        "\u{2010}\u{2011}\u{2012}\u{2013}\u{2014}\u{2015}\u{2018}\u{2019}\u{201a}\u{201b}\
         \u{201c}\u{201d}\u{201e}\u{2020}\u{2021}\u{2022}\u{2023}\u{2026}\u{2030}\u{2032}\
         \u{2033}\u{2039}\u{203a}\u{203b}\u{203c}\u{2044}",
    )); // punct
    v.extend(range(0x2070, 0x2070));
    v.extend(range(0x2074, 0x207f)); // supsub
    v.extend(range(0x2080, 0x208e));
    v.extend(range(0x2150, 0x215f)); // fractions
    v.extend(range(0x20a0, 0x20b5)); // currency
    v.extend(chars_of("\u{20b9}\u{20bd}\u{0192}"));
    v.extend(range(0x2160, 0x217f)); // roman
    v.extend(range(0x2190, 0x21ff)); // arrows
    v.extend(chars_of(
        "\u{2200}\u{2202}\u{2203}\u{2205}\u{2206}\u{2207}\u{2208}\u{2209}\u{220f}\u{2211}\
         \u{2212}\u{2213}\u{2215}\u{2217}\u{2219}\u{221a}\u{221d}\u{221e}\u{221f}\u{2220}\
         \u{2229}\u{222a}\u{222b}\u{2248}\u{2260}\u{2261}\u{2264}\u{2265}\u{226a}\u{226b}\
         \u{2282}\u{2283}\u{2295}\u{2297}\u{22a5}\u{22c5}",
    )); // math
    v.extend(range(0x2500, 0x257f)); // box
    v.extend(range(0x2580, 0x2590)); // blocks (excludes ░▒▓)
    v.extend(range(0x2594, 0x259f));
    v.extend(range(0x25a0, 0x25ff)); // geometric
    v.extend(chars_of(
        "\u{2600}\u{2601}\u{2602}\u{2603}\u{2604}\u{2605}\u{2606}\u{2609}\u{260e}\u{2610}\
         \u{2611}\u{2612}\u{2618}\u{261b}\u{261e}\u{2620}\u{2622}\u{2623}\u{262f}\u{2639}\
         \u{263a}\u{263c}\u{2640}\u{2642}",
    )); // symbols (part 1)
    v.extend(range(0x2648, 0x2653)); // symbols (part 2)
    v.extend(chars_of(
        "\u{2660}\u{2661}\u{2662}\u{2663}\u{2664}\u{2665}\u{2666}\u{2667}\u{2668}\u{2669}\
         \u{266a}\u{266b}\u{266c}\u{266d}\u{266e}\u{266f}\u{267b}",
    )); // symbols (part 3)
    v.extend(range(0x2680, 0x2685)); // symbols (part 4)
    v.extend(range(0x2690, 0x2695));
    v.extend(chars_of("\u{2699}\u{269c}\u{26a0}\u{26a1}")); // symbols (part 5)
    v.extend(chars_of(
        "\u{2708}\u{2712}\u{2713}\u{2714}\u{2715}\u{2716}\u{2717}\u{2718}\u{271a}\u{271b}\
         \u{271c}\u{2720}\u{2721}\u{2726}\u{2727}\u{2729}\u{272a}\u{272b}\u{272c}\u{272d}\
         \u{272e}\u{272f}\u{2730}\u{2731}\u{2732}\u{2733}\u{2734}\u{2735}\u{2736}\u{2737}\
         \u{2738}\u{2739}\u{273d}\u{2740}\u{2744}\u{2756}\u{2764}\u{2765}\u{2766}\u{2767}\
         \u{2794}\u{2798}\u{279c}\u{27a1}\u{27a4}\u{27b2}",
    )); // dingbats
    v.extend(range(0x2460, 0x2469)); // enclosed
    v.extend(range(0x2776, 0x277f));
    v.extend(range(0x2780, 0x2789));
    v.extend(chars_of(
        "\u{2318}\u{2325}\u{2303}\u{2324}\u{23ce}\u{232b}\u{2326}\u{21ea}\u{2423}\u{21b5}\
         \u{21b9}\u{2380}\u{2387}",
    )); // keys
    v.extend(range(0x0391, 0x03a9)); // greek
    v.extend(range(0x03b1, 0x03c9));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_block_is_present() {
        let cs = full_charset();
        assert!(cs.contains(&'A'));
        assert!(cs.contains(&'~'));
        assert!(!cs.contains(&'\u{7f}')); // DEL not in ascii 0x20..0x7e
    }

    #[test]
    fn blocks_excludes_dither() {
        let cs = full_charset();
        assert!(cs.contains(&'\u{2580}'));
        assert!(cs.contains(&'\u{2590}'));
        assert!(!cs.contains(&'\u{2591}')); // ░ excluded
        assert!(!cs.contains(&'\u{2592}'));
        assert!(!cs.contains(&'\u{2593}'));
    }
}
