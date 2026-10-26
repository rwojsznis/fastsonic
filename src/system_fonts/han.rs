//! A small set of complementary Han faces from CoreText's cascade.
//!
//! Han characters are unified in Unicode, but no face macOS uses carries
//! all of them: Hiragino Sans, which an English desktop is answered with,
//! lacks simplified characters such as 们, and PingFang lacks Japanese
//! forms such as 込. One face alone leaves the rest of a title to whatever
//! epaint finds next, which may be nothing.

/// Characters each region's faces are known to miss: shared Han first, then
/// simplified, traditional and Japanese forms.
pub(crate) const PROBES: &[char] = &[
    '中', '们', '谢', '联', '历', '单', '门', '単', '歷', '込', '気', '楽', '駅', '辺', '歩',
];

/// The names the kept faces are registered under. Each needs its own, or a
/// later face would replace the first in egui.
pub(crate) const KEYS: &[&str] = &["han", "han-2", "han-3", "han-4"];

/// What a candidate face draws, measured without any platform handles.
pub(crate) struct Coverage {
    /// A bit per entry of [`PROBES`] the face has an outline for.
    pub(crate) probes: u32,
    /// How many characters of the basic Han block the face maps.
    pub(crate) ideographs: usize,
}

/// Fewer ideographs than this is a face for the hanja in Korean names, not
/// one that runs Han text.
const RUNNING_TEXT: usize = 10_000;

/// The candidates to keep, in the order to register them: the first broad
/// face, then each face that draws a probe none before it does, up to
/// [`KEYS`]. A short hanja face never goes ahead of a broad one.
pub(crate) fn select(candidates: &[Coverage]) -> Vec<usize> {
    let mut order: Vec<_> = (0..candidates.len()).collect();
    order.sort_by_key(|&index| candidates[index].ideographs < RUNNING_TEXT);
    let mut covered = 0;
    let mut kept = Vec::new();
    for index in order {
        let coverage = candidates[index].probes;
        if coverage & 1 != 0 && coverage & !covered != 0 {
            kept.push(index);
            covered |= coverage;
            if kept.len() == KEYS.len() {
                break;
            }
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(text: &str, ideographs: usize) -> Coverage {
        Coverage {
            probes: PROBES.iter().enumerate().fold(0, |bits, (i, ch)| {
                bits | (u32::from(text.contains(*ch)) << i)
            }),
            ideographs,
        }
    }

    #[test]
    fn japanese_first_keeps_its_glyphs_and_adds_simplified_chinese() {
        let faces = [
            face("中単込気楽駅辺歩", 10767),
            face("中们谢联历单门歷", 20000),
            face("中们谢联历单门歷", 20000),
        ];
        assert_eq!(select(&faces), [0, 1]);
    }

    #[test]
    fn korean_first_puts_a_full_face_ahead_and_covers_both_gaps() {
        let faces = [
            face("中歷", 4620),
            face("中単込気楽駅辺歩", 10767),
            face("中们谢联历单门歷", 20000),
        ];
        assert_eq!(select(&faces), [1, 2]);
    }

    #[test]
    fn unreadable_redundant_and_excess_faces_are_left_out() {
        let mut faces = vec![face("", 20000), face("中们", 20000), face("中们", 20000)];
        for ch in ['谢', '联', '历', '单', '门'] {
            faces.push(face(&format!("中{ch}"), 20000));
        }
        assert_eq!(select(&faces), [1, 3, 4, 5]);
        assert!(select(&[]).is_empty());
    }

    #[test]
    fn a_short_face_still_serves_when_it_is_all_the_system_has() {
        assert_eq!(select(&[face("中歷", 4620)]), [0]);
    }

    #[test]
    fn every_probe_fits_the_coverage_bits() {
        assert!(PROBES.len() <= u32::BITS as usize);
        assert_eq!(PROBES[0], '中', "the first probe is the shared one");
    }
}
