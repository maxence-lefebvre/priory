//! The risk estimate's Beta quantiles. Each row of `priory report risk` is a Beta(1, 1) prior updated
//! by whole counts, so both of the posterior's parameters are whole numbers.

/// The share of the Beta(a, b) distribution at or below `x`, for whole `a` and `b` of at least 1:
/// the chance that a binomial of `a + b - 1` draws at `x` gets at least `a`.
pub fn beta_cdf(x: f64, a: u64, b: u64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let m = a + b - 1;
    let ln_fact: Vec<f64> = (0..=m)
        .scan(0.0, |sum, i| {
            *sum += if i == 0 { 0.0 } else { (i as f64).ln() };
            Some(*sum)
        })
        .collect();
    let (ln_x, ln_y) = (x.ln(), (1.0 - x).ln());
    let total: f64 = (a..=m)
        .map(|j| {
            let ln_choose = ln_fact[m as usize] - ln_fact[j as usize] - ln_fact[(m - j) as usize];
            (ln_choose + j as f64 * ln_x + (m - j) as f64 * ln_y).exp()
        })
        .sum();
    total.min(1.0)
}

/// The `p` quantile of Beta(a, b), by bisection on `beta_cdf`.
pub fn beta_quantile(p: f64, a: u64, b: u64) -> f64 {
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..100 {
        let middle = (low + high) / 2.0;
        if beta_cdf(middle, a, b) < p {
            low = middle;
        } else {
            high = middle;
        }
    }
    (low + high) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn percent(k: u64, n: u64) -> [String; 3] {
        [0.05, 0.5, 0.95].map(|p| format!("{:.1}", beta_quantile(p, 1 + k, 1 + n - k) * 100.0))
    }

    #[test]
    fn the_quantiles_match_the_design_notes_table() {
        let table = [
            (0, 10, ["0.5", "6.1", "23.8"]),
            (0, 20, ["0.2", "3.2", "13.3"]),
            (0, 50, ["0.1", "1.3", "5.7"]),
            (2, 41, ["2.0", "6.3", "14.2"]),
            (6, 41, ["8.1", "15.8", "26.3"]),
        ];
        for (k, n, expected) in table {
            assert_eq!(percent(k, n), expected.map(str::to_owned), "{k} of {n}");
        }
    }

    proptest! {
        #[test]
        fn quantiles_are_ordered_in_range_and_invert_the_distribution(n in 0u64..500, k in 0u64..500) {
            let k = k % (n + 1);
            let (a, b) = (1 + k, 1 + n - k);
            let quantiles = [0.05, 0.5, 0.95].map(|p| (p, beta_quantile(p, a, b)));
            prop_assert!(quantiles[0].1 <= quantiles[1].1 && quantiles[1].1 <= quantiles[2].1);
            for (p, q) in quantiles {
                prop_assert!((0.0..=1.0).contains(&q));
                prop_assert!((beta_cdf(q, a, b) - p).abs() < 1e-6, "{p} at {q}");
            }
        }
    }
}
