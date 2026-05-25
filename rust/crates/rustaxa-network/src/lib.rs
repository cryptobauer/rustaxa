//! Network pipeline types for Rustaxa ingress processing.

/// Network events passed between ingress stages.
pub mod events;

/// Network ingress worker wiring.
pub mod ingress;

/// Packet representation used by the network pipeline.
pub mod packet;
