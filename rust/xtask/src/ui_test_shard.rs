//! Splitting a UI test run across CI runners

use std::cmp::Reverse;

/// One of `count` slices of a UI test run, so CI can split it across runners; `index` is 1-based
/// to match how it is written, as in `2/3`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiTestShard {
    index: usize,
    count: usize,
}

impl UiTestShard {
    /// Whether this shard is the whole run
    pub fn is_whole_run(self) -> bool {
        self.count == 1
    }

    /// Whether this is the run's last shard
    pub fn is_last(self) -> bool {
        self.index == self.count
    }

    /// The run's other shards
    pub fn others(self) -> impl Iterator<Item = Self> {
        (1..=self.count)
            .filter(move |&index| index != self.index)
            .map(move |index| Self { index, ..self })
    }

    /// Keeps this shard's share of `items`, each paired with its estimated cost in seconds, in
    /// the order given
    ///
    /// Items are dealt out longest first, each to the shard with the least cost so far and the
    /// lowest index on a tie, so the shards finish close together, and items of equal cost go
    /// round-robin in the order given. Every shard deals the same items the same way, so the
    /// shards of one run split it into disjoint slices that cover it
    pub fn take<T>(self, items: impl IntoIterator<Item = (T, u64)>) -> Vec<T> {
        let mut items: Vec<_> = items.into_iter().enumerate().collect();

        // stable, so items of equal cost keep the order given
        items.sort_by_key(|(_, (_, cost))| Reverse(*cost));

        let mut loads = vec![0_u64; self.count];
        let mut taken = Vec::new();
        for (position, (item, cost)) in items {
            let (shard, load) = loads
                .iter_mut()
                .enumerate()
                .min_by_key(|(_, load)| **load)
                .expect("a shard count is at least 1");

            *load += cost;
            if shard == self.index - 1 {
                taken.push((position, item));
            }
        }

        taken.sort_by_key(|(position, _)| *position);
        taken.into_iter().map(|(_, item)| item).collect()
    }
}

impl std::str::FromStr for UiTestShard {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let invalid = || format!("expected a shard like 2/3, got {value:?}");

        let (index, count) = value.split_once('/').ok_or_else(invalid)?;
        let index = index.trim().parse::<usize>().map_err(|_| invalid())?;
        let count = count.trim().parse::<usize>().map_err(|_| invalid())?;

        if count == 0 || index == 0 || index > count {
            return Err(invalid());
        }

        Ok(Self { index, count })
    }
}

impl std::fmt::Display for UiTestShard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.index, self.count)
    }
}

#[cfg(test)]
mod tests {
    use super::UiTestShard;

    fn shard(value: &str) -> UiTestShard {
        value.parse().unwrap()
    }

    fn slices(count: usize, items: &[(&'static str, u64)]) -> Vec<Vec<&'static str>> {
        (1..=count)
            .map(|index| shard(&format!("{index}/{count}")).take(items.iter().copied()))
            .collect()
    }

    #[test]
    fn equal_costs_go_round_robin() {
        let items = [("a", 1), ("b", 1), ("c", 1), ("d", 1), ("e", 1)];

        assert_eq!(slices(3, &items), [vec!["a", "d"], vec!["b", "e"], vec!["c"]]);
    }

    #[test]
    fn the_longest_items_are_spread_first_and_each_slice_keeps_the_given_order() {
        let items = [("short", 10), ("long", 100), ("medium", 60), ("small", 40), ("tiny", 5)];

        // long, medium and small open the three shards, then short and tiny join small's, the
        // lightest, which still ends under the other two
        assert_eq!(
            slices(3, &items),
            [vec!["long"], vec!["medium"], vec!["short", "small", "tiny"]]
        );
    }

    #[test]
    fn slices_are_disjoint_and_cover_every_item() {
        let items: Vec<(&'static str, u64)> = ["a", "b", "c", "d", "e", "f", "g"]
            .into_iter()
            .zip([30, 5, 90, 5, 60, 0, 45])
            .collect();

        let mut covered = slices(3, &items).concat();
        covered.sort_unstable();

        let mut all: Vec<_> = items.iter().map(|(item, _)| *item).collect();
        all.sort_unstable();

        assert_eq!(covered, all);
    }

    #[test]
    fn shards_parse_only_in_range_slices() {
        assert_eq!(shard("2/3").to_string(), "2/3");
        assert!(shard(" 1 / 1 ").is_whole_run());

        for invalid in ["0/3", "4/3", "1/0", "3", "a/b", ""] {
            assert!(invalid.parse::<UiTestShard>().is_err(), "{invalid:?} parsed");
        }
    }
}
