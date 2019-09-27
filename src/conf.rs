// Config module

use crate::peer::DAGPeerList;
use crate::store::StoreType;
use futures::task::Waker;
use libcommon_rs::peer::{PeerId, PeerList};
use libconsensus::ConsensusConfiguration;
use libsignature::PublicKey;
use libsignature::SecretKey;
use libtransport::TransportType;
use std::marker::PhantomData;

pub struct DAGconfig<P, Data, SK, PK>
where
    P: PeerId,
    SK: SecretKey,
    PK: PublicKey,
{
    pub(crate) request_addr: String,
    pub(crate) reply_addr: String,
    pub(crate) shutdown: bool,
    pub(crate) transport_type: TransportType,
    pub(crate) store_type: StoreType,
    // heartbeat duration in milliseconds
    pub(crate) heartbeat: u64,
    pub(crate) waker: Option<Waker>,
    pub(crate) peers: DAGPeerList<P, PK>,
    pub(crate) creator: P,
    pub(crate) secret_key: SK,
    phantom: PhantomData<Data>,
}

impl<P, Data, SK, PK> DAGconfig<P, Data, SK, PK>
where
    P: PeerId,
    SK: SecretKey,
    PK: PublicKey,
{
    pub fn set_heartbeat(&mut self, heartbeat: u64) {
        self.heartbeat = heartbeat;
    }
    pub fn set_store_type(&mut self, store_type: StoreType) {
        self.store_type = store_type;
    }
    pub fn set_transport_type(&mut self, transport_type: TransportType) {
        self.transport_type = transport_type;
    }
    pub fn set_reply_addr(&mut self, reply_addr: String) {
        self.reply_addr = reply_addr;
    }
    pub fn set_request_addr(&mut self, request_addr: String) {
        self.request_addr = request_addr;
    }
    pub fn check_quit(&mut self) -> bool {
        self.shutdown
    }
}

impl<P, Data, SK, PK> ConsensusConfiguration<Data> for DAGconfig<P, Data, SK, PK>
where
    P: PeerId,
    SK: SecretKey,
    PK: PublicKey,
{
    fn new() -> Self {
        return DAGconfig {
            request_addr: "localhost:9000".to_string(),
            reply_addr: "localhost:12000".to_string(),
            heartbeat: 1000,
            shutdown: false,
            transport_type: TransportType::Unknown,
            store_type: StoreType::Unknown,
            waker: None,
            peers: DAGPeerList::new(),
            creator: Default::default(),
            secret_key: SK::default(),
            phantom: PhantomData,
        };
    }
}
