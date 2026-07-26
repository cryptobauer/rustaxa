#pragma once

// Rust-mode consumers retain only VoteManager's stable materialized view
// carriers. Authoritative verified-vote behavior is accessed through the
// application-owned native PBFT service; there is no C++ VerifiedVotes facade.
#include "vote_manager/verified_vote_view_types.hpp"
