use super::*;

#[test]
fn closed_clients_can_share_the_same_owner_between_threads() {
    fn shared<T: Send + Sync>() {}
    shared::<ProductionApplication>();
    shared::<ProductionNetworkClient>();
    shared::<ProductionQueryClient>();
    shared::<ProductionAdminClient>();
}
