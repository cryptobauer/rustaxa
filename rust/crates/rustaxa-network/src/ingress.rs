use std::thread::{self, JoinHandle};

use rtrb::{Consumer, PopError};

use crate::events::IncomingDagEvent;

/// Worker that consumes incoming network events for the ingress pipeline.
pub struct Ingress {
    dag_events_handle: JoinHandle<Result<IncomingDagEvent, PopError>>,
}

impl Ingress {
    /// Starts an ingress worker backed by the provided DAG event consumer.
    pub fn new(mut dag_events: Consumer<IncomingDagEvent>) -> Self {
        let handle = thread::spawn(move || dag_events.pop());

        Ingress {
            dag_events_handle: handle,
        }
    }

    /// Returns the worker thread handle.
    pub fn into_join_handle(self) -> JoinHandle<Result<IncomingDagEvent, PopError>> {
        self.dag_events_handle
    }
}
