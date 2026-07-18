use std::collections::HashMap;

/// Run-level verdict of a test: derived from all its executions in one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Fail,
    /// Both passing and failing executions in the same run (retry flip).
    Mixed,
}

impl Verdict {
    /// None means only skipped executions: excluded from scoring.
    pub fn from_counts(passes: i64, fails: i64) -> Option<Verdict> {
        match (passes > 0, fails > 0) {
            (true, true) => Some(Verdict::Mixed),
            (true, false) => Some(Verdict::Pass),
            (false, true) => Some(Verdict::Fail),
            (false, false) => None,
        }
    }
}

pub struct WindowEntry {
    pub sha: String,
    pub verdict: Verdict,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct Score {
    pub score: u32,
    pub flip_shas: usize,
    pub flips: usize,
}

/// Scores a test over its verdict window (chronological order).
///
/// flip_shas: distinct SHAs with both pass and fail evidence (Mixed counts alone).
/// flips: adjacent pass/fail transitions, Mixed excluded (already counted via flip_shas).
/// Flaky iff flip_shas >= 1 or flips >= 2; a single cross-SHA flip is an honest break/fix.
pub fn score(window: &[WindowEntry]) -> Score {
    let mut by_sha: HashMap<&str, (bool, bool)> = HashMap::new();
    for e in window {
        let (pass, fail) = by_sha.entry(&e.sha).or_default();
        match e.verdict {
            Verdict::Pass => *pass = true,
            Verdict::Fail => *fail = true,
            Verdict::Mixed => {
                *pass = true;
                *fail = true;
            }
        }
    }
    let flip_shas = by_sha
        .values()
        .filter(|(pass, fail)| *pass && *fail)
        .count();
    let flips = window
        .iter()
        .filter(|e| e.verdict != Verdict::Mixed)
        .map(|e| e.verdict)
        .collect::<Vec<_>>()
        .windows(2)
        .filter(|w| w[0] != w[1])
        .count();
    let n = window.len();
    let score = if flip_shas >= 1 || flips >= 2 {
        let sha_part = 0.6 * (flip_shas.min(3) as f64) / 3.0;
        let flip_part = 0.4 * flips as f64 / (n.saturating_sub(1).max(1)) as f64;
        (100.0 * (sha_part + flip_part).min(1.0)).round() as u32
    } else {
        0
    };
    Score {
        score,
        flip_shas,
        flips,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(sha: &str, verdict: Verdict) -> WindowEntry {
        WindowEntry {
            sha: sha.into(),
            verdict,
        }
    }

    #[test]
    fn same_sha_flip_across_runs_is_flaky() {
        let s = score(&[entry("a", Verdict::Fail), entry("a", Verdict::Pass)]);
        assert_eq!(s.flip_shas, 1);
        assert_eq!(s.flips, 1);
        assert!(s.score >= 20, "score = {}", s.score);
    }

    #[test]
    fn mixed_verdict_alone_is_flaky() {
        let s = score(&[entry("a", Verdict::Mixed)]);
        assert_eq!(s.flip_shas, 1);
        assert_eq!(s.flips, 0);
        assert_eq!(s.score, 20);
    }

    #[test]
    fn honest_regression_scores_zero() {
        let s = score(&[
            entry("a", Verdict::Pass),
            entry("b", Verdict::Pass),
            entry("c", Verdict::Fail),
            entry("d", Verdict::Fail),
        ]);
        assert_eq!(
            s,
            Score {
                score: 0,
                flip_shas: 0,
                flips: 1
            }
        );
    }

    #[test]
    fn stable_pass_scores_zero() {
        let s = score(&[entry("a", Verdict::Pass), entry("b", Verdict::Pass)]);
        assert_eq!(s.score, 0);
    }

    #[test]
    fn cross_sha_alternation_is_flaky() {
        let s = score(&[
            entry("a", Verdict::Pass),
            entry("b", Verdict::Fail),
            entry("c", Verdict::Pass),
            entry("d", Verdict::Fail),
        ]);
        assert_eq!(s.flip_shas, 0);
        assert_eq!(s.flips, 3);
        assert_eq!(s.score, 40);
    }

    #[test]
    fn score_is_capped_at_100() {
        let entries: Vec<WindowEntry> = (0..10)
            .flat_map(|i| {
                let sha = format!("s{i}");
                [entry(&sha, Verdict::Pass), entry(&sha, Verdict::Fail)]
            })
            .collect();
        assert_eq!(score(&entries).score, 100);
    }
}
