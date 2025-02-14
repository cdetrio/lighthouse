use beacon_chain::{
    fork_choice::OptimizedLMDGhost, slot_clock::SystemTimeSlotClock, store::Store, BeaconChain,
    BeaconChainTypes,
};
use fork_choice::ForkChoice;
use slog::{info, Logger};
use slot_clock::SlotClock;
use std::marker::PhantomData;
use std::sync::Arc;
use tree_hash::TreeHash;
use types::{test_utils::TestingBeaconStateBuilder, BeaconBlock, ChainSpec, EthSpec, Hash256};

/// The number initial validators when starting the `Minimal`.
const TESTNET_VALIDATOR_COUNT: usize = 16;

/// Provides a new, initialized `BeaconChain`
pub trait InitialiseBeaconChain<T: BeaconChainTypes> {
    fn initialise_beacon_chain(
        store: Arc<T::Store>,
        spec: ChainSpec,
        log: Logger,
    ) -> BeaconChain<T> {
        maybe_load_from_store_for_testnet::<_, T::Store, T::EthSpec>(store, spec, log)
    }
}

#[derive(Clone)]
pub struct ClientType<S: Store, E: EthSpec> {
    _phantom_t: PhantomData<S>,
    _phantom_u: PhantomData<E>,
}

impl<S: Store, E: EthSpec + Clone> BeaconChainTypes for ClientType<S, E> {
    type Store = S;
    type SlotClock = SystemTimeSlotClock;
    type ForkChoice = OptimizedLMDGhost<S, E>;
    type EthSpec = E;
}
impl<T: Store, E: EthSpec, X: BeaconChainTypes> InitialiseBeaconChain<X> for ClientType<T, E> {}

/// Loads a `BeaconChain` from `store`, if it exists. Otherwise, create a new chain from genesis.
fn maybe_load_from_store_for_testnet<T, U: Store, V: EthSpec>(
    store: Arc<U>,
    spec: ChainSpec,
    log: Logger,
) -> BeaconChain<T>
where
    T: BeaconChainTypes<Store = U>,
    T::ForkChoice: ForkChoice<U>,
{
    if let Ok(Some(beacon_chain)) = BeaconChain::from_store(store.clone(), spec.clone()) {
        info!(
            log,
            "Loaded BeaconChain from store";
            "slot" => beacon_chain.head().beacon_state.slot,
            "best_slot" => beacon_chain.best_slot(),
        );

        beacon_chain
    } else {
        info!(log, "Initializing new BeaconChain from genesis");
        let state_builder = TestingBeaconStateBuilder::from_default_keypairs_file_if_exists(
            TESTNET_VALIDATOR_COUNT,
            &spec,
        );
        let (genesis_state, _keypairs) = state_builder.build();

        let mut genesis_block = BeaconBlock::empty(&spec);
        genesis_block.state_root = Hash256::from_slice(&genesis_state.tree_hash_root());

        // Slot clock
        let slot_clock = T::SlotClock::new(
            spec.genesis_slot,
            genesis_state.genesis_time,
            spec.seconds_per_slot,
        );
        // Choose the fork choice
        let fork_choice = T::ForkChoice::new(store.clone());

        // Genesis chain
        //TODO: Handle error correctly
        BeaconChain::from_genesis(
            store,
            slot_clock,
            genesis_state,
            genesis_block,
            spec,
            fork_choice,
        )
        .expect("Terminate if beacon chain generation fails")
    }
}
