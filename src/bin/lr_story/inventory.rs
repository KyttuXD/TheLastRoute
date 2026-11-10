//! What the player carries: nine hotbar slots (keys 1-9) and an unlimited bag behind them. Items stack up to a
//! per-item limit. No drawing and no input here; `play.rs` owns one and keeps its distinct held ids in sync.

pub const SLOTS: usize = 9;

/// A pile of one item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stack {
    pub item: String,
    pub count: u32,
}

/// Where a stack sits: a hotbar slot or a place in the bag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Slot(usize),
    Bag(usize),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    pub slots: [Option<Stack>; SLOTS],
    pub bag: Vec<Stack>,
    pub selected: usize,
}

impl Inventory {
    pub fn new() -> Inventory {
        Inventory::default()
    }

    /// Adds `count` of `item`: tops up the stacks of that item first (hotbar, then bag), then fills the first empty
    /// hotbar slot, then the bag. Never loses items.
    pub fn add(&mut self, item: &str, count: u32, max_stack: u32) {
        let max = max_stack.max(1);
        let mut left = count;
        for s in self.slots.iter_mut().flatten().chain(self.bag.iter_mut()) {
            if left == 0 {
                return;
            }
            if s.item == item && s.count < max {
                let put = (max - s.count).min(left);
                s.count += put;
                left -= put;
            }
        }
        while left > 0 {
            let put = left.min(max);
            left -= put;
            let stack = Stack { item: item.to_string(), count: put };
            match self.slots.iter_mut().find(|s| s.is_none()) {
                Some(empty) => *empty = Some(stack),
                None => self.bag.push(stack),
            }
        }
    }

    /// Removes one `item` from the stack at `place` when that stack holds it (an emptied stack clears its slot or leaves
    /// the bag), else one from anywhere (`remove`). Using an item takes it from the stack the player chose.
    pub fn remove_one_at(&mut self, place: Place, item: &str) -> bool {
        match place {
            Place::Slot(i) => {
                if let Some(Some(s)) = self.slots.get_mut(i) {
                    if s.item == item && s.count > 0 {
                        s.count -= 1;
                        if s.count == 0 {
                            self.slots[i] = None;
                        }
                        return true;
                    }
                }
            }
            Place::Bag(j) => {
                if let Some(s) = self.bag.get_mut(j) {
                    if s.item == item && s.count > 0 {
                        s.count -= 1;
                        if s.count == 0 {
                            self.bag.remove(j);
                        }
                        return true;
                    }
                }
            }
        }
        self.remove(item, 1)
    }

    /// Removes `count` of `item` (hotbar first, then bag). False, and nothing removed, when fewer are held.
    pub fn remove(&mut self, item: &str, count: u32) -> bool {
        if self.count(item) < count {
            return false;
        }
        let mut left = count;
        for slot in self.slots.iter_mut() {
            if left == 0 {
                break;
            }
            if let Some(s) = slot {
                if s.item == item {
                    let take = s.count.min(left);
                    s.count -= take;
                    left -= take;
                    if s.count == 0 {
                        *slot = None;
                    }
                }
            }
        }
        for s in self.bag.iter_mut() {
            if left == 0 {
                break;
            }
            if s.item == item {
                let take = s.count.min(left);
                s.count -= take;
                left -= take;
            }
        }
        self.bag.retain(|s| s.count > 0);
        true
    }

    pub fn count(&self, item: &str) -> u32 {
        self.slots.iter().flatten().chain(self.bag.iter()).filter(|s| s.item == item).map(|s| s.count).sum()
    }

    pub fn has(&self, item: &str) -> bool {
        self.count(item) > 0
    }

    pub fn select(&mut self, slot: usize) {
        if slot < SLOTS {
            self.selected = slot;
        }
    }

    /// Moves the selection by `delta` slots, wrapping around.
    pub fn scroll(&mut self, delta: i32) {
        self.selected = (self.selected as i32 + delta).rem_euclid(SLOTS as i32) as usize;
    }

    pub fn selected_item(&self) -> Option<&str> {
        self.slots[self.selected.min(SLOTS - 1)].as_ref().map(|s| s.item.as_str())
    }

    pub fn swap(&mut self, a: usize, b: usize) {
        if a < SLOTS && b < SLOTS {
            self.slots.swap(a, b);
        }
    }

    /// Puts bag stack `bag_index` into hotbar slot `slot`; what was there goes to the bag in its place.
    pub fn move_bag_to_slot(&mut self, bag_index: usize, slot: usize) {
        if bag_index >= self.bag.len() || slot >= SLOTS {
            return;
        }
        let moved = self.bag.remove(bag_index);
        if let Some(old) = self.slots[slot].replace(moved) {
            self.bag.insert(bag_index, old);
        }
    }

    /// Distinct held item ids: hotbar order, then bag.
    pub fn held(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for s in self.slots.iter().flatten().chain(self.bag.iter()) {
            if !out.contains(&s.item) {
                out.push(s.item.clone());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_fills_stacks_then_slots_then_the_bag() {
        let mut inv = Inventory::new();
        inv.add("tonic", 3, 5);
        assert_eq!(inv.slots[0], Some(Stack { item: "tonic".into(), count: 3 }));
        inv.add("tonic", 4, 5); // tops up to 5, the other 2 open slot 1
        assert_eq!(inv.slots[0].as_ref().unwrap().count, 5);
        assert_eq!(inv.slots[1].as_ref().unwrap().count, 2);
        for i in 0..7 {
            inv.add(&format!("k{i}"), 1, 1);
        }
        assert!(inv.slots.iter().all(|s| s.is_some()));
        inv.add("extra", 1, 1);
        assert_eq!(inv.bag, vec![Stack { item: "extra".into(), count: 1 }]);
        assert_eq!(inv.count("tonic"), 7);
    }

    #[test]
    fn stack_limits_split_big_adds_and_never_lose_items() {
        let mut inv = Inventory::new();
        inv.add("arrow", 25, 10);
        let counts: Vec<u32> = inv.slots.iter().flatten().map(|s| s.count).collect();
        assert_eq!(counts, vec![10, 10, 5]);
        assert_eq!(inv.count("arrow"), 25);
        inv.add("arrow", 5, 0); // a limit of 0 acts as 1
        assert_eq!(inv.count("arrow"), 30);
    }

    /// Using an item takes it from the stack the player chose, not from the first stack of that item.
    #[test]
    fn remove_one_at_takes_from_the_chosen_stack() {
        let mut inv = Inventory::new();
        inv.add("tonic", 9, 9);
        inv.add("tonic", 3, 9);
        assert_eq!((inv.slots[0].as_ref().unwrap().count, inv.slots[1].as_ref().unwrap().count), (9, 3));
        assert!(inv.remove_one_at(Place::Slot(1), "tonic"));
        assert_eq!((inv.slots[0].as_ref().unwrap().count, inv.slots[1].as_ref().unwrap().count), (9, 2));
        inv.bag.push(Stack { item: "tonic".into(), count: 1 });
        assert!(inv.remove_one_at(Place::Bag(0), "tonic"));
        assert!(inv.bag.is_empty(), "an emptied bag stack leaves the bag");
        // a place that does not hold the item falls back to anywhere
        assert!(inv.remove_one_at(Place::Slot(5), "tonic"));
        assert_eq!(inv.count("tonic"), 10);
    }

    #[test]
    fn a_key_item_takes_a_second_slot() {
        let mut inv = Inventory::new();
        inv.add("key", 1, 1);
        inv.add("key", 1, 1);
        assert_eq!(inv.slots[0].as_ref().unwrap().count, 1);
        assert_eq!(inv.slots[1].as_ref().unwrap().count, 1);
        assert_eq!(inv.held(), vec!["key".to_string()]);
    }

    #[test]
    fn remove_takes_from_the_hotbar_before_the_bag_and_empty_stacks_clear() {
        let mut inv = Inventory::new();
        inv.bag.push(Stack { item: "a".into(), count: 2 });
        inv.slots[3] = Some(Stack { item: "a".into(), count: 1 });
        assert!(inv.remove("a", 1));
        assert!(inv.slots[3].is_none(), "empty stack clears its slot");
        assert_eq!(inv.bag[0].count, 2);
        assert!(!inv.remove("a", 3), "too few: nothing removed");
        assert_eq!(inv.count("a"), 2);
        assert!(inv.remove("a", 2));
        assert!(inv.bag.is_empty());
        assert!(!inv.has("a"));
        assert!(!inv.remove("a", 1));
    }

    #[test]
    fn select_and_scroll_wrap() {
        let mut inv = Inventory::new();
        inv.select(4);
        assert_eq!(inv.selected, 4);
        inv.select(9);
        assert_eq!(inv.selected, 4, "out of range ignored");
        inv.select(0);
        inv.scroll(-1);
        assert_eq!(inv.selected, 8);
        inv.scroll(1);
        assert_eq!(inv.selected, 0);
        inv.scroll(10);
        assert_eq!(inv.selected, 1);
        inv.select(0);
        inv.add("x", 1, 1);
        assert_eq!(inv.selected_item(), Some("x"));
        inv.select(1);
        assert_eq!(inv.selected_item(), None);
    }

    #[test]
    fn swap_and_move_bag_to_slot_swap_with_what_was_there() {
        let mut inv = Inventory::new();
        inv.add("a", 1, 1);
        inv.add("b", 1, 1);
        inv.swap(0, 1);
        assert_eq!(inv.slots[0].as_ref().unwrap().item, "b");
        inv.swap(0, 5);
        assert!(inv.slots[0].is_none() && inv.slots[5].as_ref().unwrap().item == "b");
        inv.bag.push(Stack { item: "c".into(), count: 2 });
        inv.bag.push(Stack { item: "d".into(), count: 1 });
        inv.move_bag_to_slot(0, 5); // swaps with b
        assert_eq!(inv.slots[5].as_ref().unwrap().item, "c");
        assert_eq!(inv.bag[0].item, "b");
        inv.move_bag_to_slot(1, 8); // empty slot: just moves
        assert_eq!(inv.slots[8].as_ref().unwrap().item, "d");
        assert_eq!(inv.bag.len(), 1);
        inv.move_bag_to_slot(7, 0); // out of range: nothing
        assert_eq!(inv.bag.len(), 1);
    }

    #[test]
    fn held_lists_distinct_ids_hotbar_then_bag() {
        let mut inv = Inventory::new();
        inv.slots[4] = Some(Stack { item: "z".into(), count: 1 });
        inv.slots[1] = Some(Stack { item: "y".into(), count: 1 });
        inv.bag.push(Stack { item: "z".into(), count: 1 });
        inv.bag.push(Stack { item: "w".into(), count: 1 });
        assert_eq!(inv.held(), vec!["y".to_string(), "z".to_string(), "w".to_string()]);
    }
}
