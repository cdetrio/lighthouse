use crate::Store;
use std::borrow::Cow;
use std::sync::Arc;
use types::{BeaconBlock, BeaconState, BeaconStateError, EthSpec, Hash256, Slot};

#[derive(Clone)]
pub struct StateRootsIterator<'a, T: EthSpec, U> {
    store: Arc<U>,
    beacon_state: Cow<'a, BeaconState<T>>,
    slot: Slot,
}

impl<'a, T: EthSpec, U: Store> StateRootsIterator<'a, T, U> {
    /// Create a new iterator over all blocks in the given `beacon_state` and prior states.
    pub fn new(store: Arc<U>, beacon_state: &'a BeaconState<T>, start_slot: Slot) -> Self {
        Self {
            store,
            beacon_state: Cow::Borrowed(beacon_state),
            slot: start_slot,
        }
    }
}

impl<'a, T: EthSpec, U: Store> Iterator for StateRootsIterator<'a, T, U> {
    type Item = (Hash256, Slot);

    fn next(&mut self) -> Option<Self::Item> {
        if (self.slot == 0) || (self.slot > self.beacon_state.slot) {
            return None;
        }

        self.slot -= 1;

        match self.beacon_state.get_state_root(self.slot) {
            Ok(root) => Some((*root, self.slot)),
            Err(BeaconStateError::SlotOutOfBounds) => {
                // Read a `BeaconState` from the store that has access to prior historical root.
                let beacon_state: BeaconState<T> = {
                    let new_state_root = self.beacon_state.get_oldest_state_root().ok()?;

                    self.store.get(&new_state_root).ok()?
                }?;

                self.beacon_state = Cow::Owned(beacon_state);

                let root = self.beacon_state.get_state_root(self.slot).ok()?;

                Some((*root, self.slot))
            }
            _ => None,
        }
    }
}

#[derive(Clone)]
/// Extends `BlockRootsIterator`, returning `BeaconBlock` instances, instead of their roots.
pub struct BlockIterator<'a, T: EthSpec, U> {
    roots: BlockRootsIterator<'a, T, U>,
}

impl<'a, T: EthSpec, U: Store> BlockIterator<'a, T, U> {
    /// Create a new iterator over all blocks in the given `beacon_state` and prior states.
    pub fn new(store: Arc<U>, beacon_state: &'a BeaconState<T>, start_slot: Slot) -> Self {
        Self {
            roots: BlockRootsIterator::new(store, beacon_state, start_slot),
        }
    }
}

impl<'a, T: EthSpec, U: Store> Iterator for BlockIterator<'a, T, U> {
    type Item = BeaconBlock;

    fn next(&mut self) -> Option<Self::Item> {
        let (root, _slot) = self.roots.next()?;
        self.roots.store.get(&root).ok()?
    }
}

/// Iterates backwards through block roots.
///
/// Uses the `latest_block_roots` field of `BeaconState` to as the source of block roots and will
/// perform a lookup on the `Store` for a prior `BeaconState` if `latest_block_roots` has been
/// exhausted.
///
/// Returns `None` for roots prior to genesis or when there is an error reading from `Store`.
#[derive(Clone)]
pub struct BlockRootsIterator<'a, T: EthSpec, U> {
    store: Arc<U>,
    beacon_state: Cow<'a, BeaconState<T>>,
    slot: Slot,
}

impl<'a, T: EthSpec, U: Store> BlockRootsIterator<'a, T, U> {
    /// Create a new iterator over all block roots in the given `beacon_state` and prior states.
    pub fn new(store: Arc<U>, beacon_state: &'a BeaconState<T>, start_slot: Slot) -> Self {
        Self {
            slot: start_slot,
            beacon_state: Cow::Borrowed(beacon_state),
            store,
        }
    }
}

impl<'a, T: EthSpec, U: Store> Iterator for BlockRootsIterator<'a, T, U> {
    type Item = (Hash256, Slot);

    fn next(&mut self) -> Option<Self::Item> {
        if (self.slot == 0) || (self.slot > self.beacon_state.slot) {
            return None;
        }

        self.slot -= 1;

        match self.beacon_state.get_block_root(self.slot) {
            Ok(root) => Some((*root, self.slot)),
            Err(BeaconStateError::SlotOutOfBounds) => {
                // Read a `BeaconState` from the store that has access to prior historical root.
                let beacon_state: BeaconState<T> = {
                    // Load the earlier state from disk. Skip forward one slot, because a state
                    // doesn't return it's own state root.
                    let new_state_root = self.beacon_state.get_oldest_state_root().ok()?;

                    self.store.get(&new_state_root).ok()?
                }?;

                self.beacon_state = Cow::Owned(beacon_state);

                let root = self.beacon_state.get_block_root(self.slot).ok()?;

                Some((*root, self.slot))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::MemoryStore;
    use types::{test_utils::TestingBeaconStateBuilder, Keypair, MainnetEthSpec};

    fn get_state<T: EthSpec>() -> BeaconState<T> {
        let builder = TestingBeaconStateBuilder::from_single_keypair(
            0,
            &Keypair::random(),
            &T::default_spec(),
        );
        let (state, _keypairs) = builder.build();
        state
    }

    #[test]
    fn block_root_iter() {
        let store = Arc::new(MemoryStore::open());
        let slots_per_historical_root = MainnetEthSpec::slots_per_historical_root();

        let mut state_a: BeaconState<MainnetEthSpec> = get_state();
        let mut state_b: BeaconState<MainnetEthSpec> = get_state();

        state_a.slot = Slot::from(slots_per_historical_root);
        state_b.slot = Slot::from(slots_per_historical_root * 2);

        let mut hashes = (0..).into_iter().map(|i| Hash256::from(i));

        for root in &mut state_a.latest_block_roots[..] {
            *root = hashes.next().unwrap()
        }
        for root in &mut state_b.latest_block_roots[..] {
            *root = hashes.next().unwrap()
        }

        let state_a_root = hashes.next().unwrap();
        state_b.latest_state_roots[0] = state_a_root;
        store.put(&state_a_root, &state_a).unwrap();

        let iter = BlockRootsIterator::new(store.clone(), &state_b, state_b.slot - 1);

        assert!(
            iter.clone().find(|(_root, slot)| *slot == 0).is_some(),
            "iter should contain zero slot"
        );

        let mut collected: Vec<(Hash256, Slot)> = iter.collect();
        collected.reverse();

        let expected_len = 2 * MainnetEthSpec::slots_per_historical_root() - 1;

        assert_eq!(collected.len(), expected_len);

        for i in 0..expected_len {
            assert_eq!(collected[i].0, Hash256::from(i as u64));
        }
    }

    #[test]
    fn state_root_iter() {
        let store = Arc::new(MemoryStore::open());
        let slots_per_historical_root = MainnetEthSpec::slots_per_historical_root();

        let mut state_a: BeaconState<MainnetEthSpec> = get_state();
        let mut state_b: BeaconState<MainnetEthSpec> = get_state();

        state_a.slot = Slot::from(slots_per_historical_root);
        state_b.slot = Slot::from(slots_per_historical_root * 2);

        let mut hashes = (0..).into_iter().map(|i| Hash256::from(i));

        for slot in 0..slots_per_historical_root {
            state_a
                .set_state_root(Slot::from(slot), hashes.next().unwrap())
                .expect(&format!("should set state_a slot {}", slot));
        }
        for slot in slots_per_historical_root..slots_per_historical_root * 2 {
            state_b
                .set_state_root(Slot::from(slot), hashes.next().unwrap())
                .expect(&format!("should set state_b slot {}", slot));
        }

        /*
        for root in &mut state_a.latest_state_roots[..] {
            state_a.set_state_root(slots.next().unwrap(), hashes.next().unwrap());
            // *root = hashes.next().unwrap()
        }
        for root in &mut state_b.latest_state_roots[..] {
            state_b.set_state_root(slots.next().unwrap(), hashes.next().unwrap());
            *root = hashes.next().unwrap()
        }
        */

        let state_a_root = Hash256::from(slots_per_historical_root as u64);
        let state_b_root = Hash256::from(slots_per_historical_root as u64 * 2);

        store.put(&state_a_root, &state_a).unwrap();
        store.put(&state_b_root, &state_b).unwrap();

        let iter = StateRootsIterator::new(store.clone(), &state_b, state_b.slot - 1);

        assert!(
            iter.clone().find(|(_root, slot)| *slot == 0).is_some(),
            "iter should contain zero slot"
        );

        let mut collected: Vec<(Hash256, Slot)> = iter.collect();
        collected.reverse();

        let expected_len = MainnetEthSpec::slots_per_historical_root() * 2 - 1;

        assert_eq!(collected.len(), expected_len, "collection length incorrect");

        for i in 0..expected_len {
            let (hash, slot) = collected[i];

            assert_eq!(slot, i as u64, "slot mismatch at {}: {} vs {}", i, slot, i);

            assert_eq!(hash, Hash256::from(i as u64), "hash mismatch at {}", i);
        }
    }
}
