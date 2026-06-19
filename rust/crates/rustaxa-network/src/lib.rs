//! Rust network ingress components for Taraxa.
//!
//! The crate provides packet metadata, peer/session tracking, bounded ingress
//! queueing, and event types used while the Rust network rewrite is wired into
//! the existing node.

#![warn(missing_docs)]

pub mod egress;
/// Network event types shared between pipeline stages.
pub mod events;
/// Early packet filtering helpers.
pub mod filter;
/// Consumer-side ingress worker for queued network events.
pub mod ingress;
/// Producer-side network facade for packet ingestion and peer handling.
pub mod network;
/// Packet metadata and payload storage.
pub mod packet;
/// Peer registry and session tracking.
pub mod peers;
