//! Pure filter: which hunters count for the runner's own cues (danger readout, heartbeat, chase cue).
//! Hunters can target bots on Hard and Nightmare; those must never alarm the player.

/// Keep only the items whose target is the runner.
pub fn runner_targeting<T>(items: &[T], targets_runner: impl Fn(&T) -> bool) -> Vec<&T> {
    items.iter().filter(|i| targets_runner(i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_runner_targets() {
        let v = [(1, true), (2, false), (3, true)];
        let out = runner_targeting(&v, |x| x.1);
        assert_eq!(out.iter().map(|x| x.0).collect::<Vec<_>>(), vec![1, 3]);
    }

    #[test]
    fn bot_only_list_is_empty() {
        assert!(runner_targeting(&[(1, false)], |x| x.1).is_empty());
    }
}
