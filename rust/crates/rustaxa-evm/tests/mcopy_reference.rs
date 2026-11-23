//! Check that later instruction phases include earlier phases.

use rustaxa_evm::profile::{TaraxaPhase, TaraxaProfile};

/// Pins the cumulative phase API while preserving the legacy boolean creator.
#[test]
fn explicit_phase_api_is_cumulative_and_compatible() {
    let californicum = TaraxaProfile::for_phase(TaraxaPhase::Californicum);
    let ficus = TaraxaProfile::for_phase(TaraxaPhase::Ficus);
    let cacti = TaraxaProfile::for_phase(TaraxaPhase::Cacti);

    assert_eq!(TaraxaProfile::new(false), californicum);
    assert_eq!(TaraxaProfile::new(true), cacti);
    assert_eq!(californicum.phase(), TaraxaPhase::Californicum);
    assert_eq!(ficus.phase(), TaraxaPhase::Ficus);
    assert_eq!(cacti.phase(), TaraxaPhase::Cacti);
    assert!(!californicum.ficus());
    assert!(ficus.ficus());
    assert!(cacti.ficus());
    assert!(!californicum.cacti());
    assert!(!ficus.cacti());
    assert!(cacti.cacti());
}
