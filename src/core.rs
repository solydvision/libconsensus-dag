use crate::conf::DAGconfig;
use crate::errors::Result;
use crate::event::Event;
use crate::flag_table::creator_flag_table_fmt;
use crate::flag_table::flag_table_fmt;
use crate::flag_table::FlagTable;
use crate::flag_table::{min_frame, open_merge_flag_table, strict_merge_flag_table};
use crate::lamport_time::LamportTime;
use crate::peer::FrameNumber;
use crate::store::DAGstore;
use crate::store_sled::SledStore;
use crate::transactions::InternalTransaction;
use core::mem::swap;
use libcommon_rs::data::DataType;
use libcommon_rs::peer::Peer;
use libcommon_rs::peer::PeerId;
use libcommon_rs::peer::PeerList;
use libconsensus::errors::Result as BaseResult;
use libhash_sha3::Hash as EventHash;
use libsignature::PublicKey;
use libsignature::SecretKey;
use libsignature::Signature;
use std::cmp::Ordering;
use std::sync::Arc;
use std::sync::RwLock;

pub(crate) struct DAGcore<P, Data, SK, PK, Sig>
where
    Data: DataType,
    P: PeerId,
    SK: SecretKey,
    PK: PublicKey,
    Sig: Signature<Hash = EventHash, PublicKey = PK, SecretKey = SK>,
{
    pub(crate) conf: Arc<RwLock<DAGconfig<P, Data, SK, PK>>>,
    pub(crate) store: Arc<RwLock<dyn DAGstore<Data, P, PK, Sig>>>,
    tx_pool: Vec<Data>,
    internal_tx_pool: Vec<InternalTransaction<P, PK>>,
    lamport_time: LamportTime,
    pub(crate) current_frame: Option<FrameNumber>,
    pub(crate) current_event: Option<usize>,
    pub(crate) current_tx: Option<usize>,
    pub(crate) last_finalised_frame: Option<FrameNumber>,
    me_a: String,
    me_b: String,
    pub(crate) shutdown: bool,
}

impl<P, Data, SK, PK, Sig> DAGcore<P, Data, SK, PK, Sig>
where
    P: PeerId,
    Data: DataType,
    SK: SecretKey,
    PK: PublicKey,
    Sig: Signature<Hash = EventHash, PublicKey = PK, SecretKey = SK>,
{
    // Defines maximum number of transactions in a single event
    const TRANSACTIONS_LIMIT: usize = 16000;

    pub(crate) fn new(conf: DAGconfig<P, Data, SK, PK>) -> DAGcore<P, Data, SK, PK, Sig> {
        let store_type = conf.store_type.clone();
        let store = {
            match store_type {
                libcommon_rs::store::StoreType::Unknown => panic!("unknown DAG store"),
                libcommon_rs::store::StoreType::Sled => {
                    let path_buf = conf.store_dir.join(conf.creator.to_string());
                    <SledStore as DAGstore<Data, P, PK, Sig>>::new(path_buf.as_path()).unwrap()
                }
            }
        };
        let me_a = format!("{} {}", conf.get_creator(), conf.reply_addr.clone());
        let me_b = format!("{} {}", conf.get_creator(), conf.request_addr.clone());
        let core = DAGcore {
            conf: Arc::new(RwLock::new(conf)),
            store: Arc::new(RwLock::new(store)),
            tx_pool: Vec::with_capacity(1),
            internal_tx_pool: Vec::with_capacity(1),
            lamport_time: LamportTime::default(),
            current_frame: None,
            current_event: Some(0),
            current_tx: Some(0),
            last_finalised_frame: None,
            me_a,
            me_b,
            shutdown: false,
        };
        // Set creator for peer list
        {
            let mut cfg = core.conf.write().unwrap();
            let creator = cfg.get_creator();
            cfg.peers.set_creator(creator);
        }
        // Create leaf events
        let peers = { core.conf.read().unwrap().peers.clone() };
        for peer in peers.iter() {
            let mut event: Event<Data, P, PK, Sig> = Event::new(
                peer.get_id(),
                peer.get_height(),
                EventHash::default(),
                EventHash::default(),
                peer.get_lamport_time(),
                [].to_vec(),
                [].to_vec(),
            );
            let ex = event.event_hash().unwrap();
            let mut ft = FlagTable::new();
            ft.insert(ex.clone(), 0);
            {
                let mut store = core.store.write().unwrap();
                store.set_event(event).unwrap();
                store.set_flag_table(&ex, &ft).unwrap();
            }
        }
        core
    }
    pub fn check_quit(&self) -> bool {
        self.shutdown
    }
    pub(crate) fn set_shutdown(&mut self, shutdown: bool) {
        self.shutdown = shutdown;
    }
    pub(crate) fn me_a(&self) -> String {
        self.me_a.clone()
    }
    pub(crate) fn me_b(&self) -> String {
        self.me_b.clone()
    }
    pub(crate) fn get_lamport_time(&self) -> LamportTime {
        self.lamport_time
    }
    pub(crate) fn get_next_lamport_time(&mut self) -> LamportTime {
        self.lamport_time += 1;
        self.lamport_time
    }
    pub(crate) fn add_transaction(&mut self, data: Data) -> BaseResult<()> {
        self.tx_pool.push(data);
        Ok(())
    }
    pub(crate) fn next_transactions(&mut self) -> Vec<Data> {
        let mut len = self.tx_pool.len();
        if len > Self::TRANSACTIONS_LIMIT {
            len = Self::TRANSACTIONS_LIMIT;
        }
        let mut new_trx = self.tx_pool.split_off(len);
        swap(&mut self.tx_pool, &mut new_trx);
        new_trx
    }
    // FIXME: reserved for DAG1
    //    pub(crate) fn add_internal_transaction(
    //        &mut self,
    //        tx: InternalTransaction<P, PK>,
    //    ) -> Result<()> {
    //        self.internal_tx_pool.push(tx);
    //        Ok(())
    //    }
    pub(crate) fn next_internal_transactions(&mut self) -> Vec<InternalTransaction<P, PK>> {
        let mut len = self.internal_tx_pool.len();
        if len > Self::TRANSACTIONS_LIMIT {
            len = Self::TRANSACTIONS_LIMIT;
        }
        let mut new_trx = self.internal_tx_pool.split_off(len);
        swap(&mut self.internal_tx_pool, &mut new_trx);
        new_trx
    }
    pub(crate) fn update_lamport_time(&mut self, time: LamportTime) {
        if self.lamport_time < time {
            self.lamport_time = time;
        }
    }
    pub(crate) fn check_event(&self, event: &Event<Data, P, PK, Sig>) -> Result<bool> {
        // FIXME: implement event verification:
        // - self-parеnt must be the last known event of the creator with height one minus height of the event
        // - all signatures must be verified positively
        for (signatory, signature) in event.signatures.iter() {
            let peer = { self.conf.read().unwrap().peers.find_peer(signatory)? };
            let res = signature.verify(event.get_hash(), peer.get_public_key())?;
            if !res {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(crate) fn insert_event(&mut self, mut event: Event<Data, P, PK, Sig>) -> Result<bool> {
        let event_hash = event.event_hash()?;
        let self_parent = event.self_parent;
        let other_parent = event.other_parent;
        let (self_parent_event, other_parent_event, self_parent_ft, other_parent_ft) = {
            let store = self.store.read().unwrap();
            (
                store.get_event(&self_parent)?,
                store.get_event(&other_parent)?,
                store.get_flag_table(&self_parent)?,
                store.get_flag_table(&other_parent)?,
            )
        };
        debug!("{}: * event and ft read", self.me_a());
        let root: bool; // = false;
        let frame: FrameNumber /* FrameNumber::default() */ =
            match self_parent_event.frame_number.cmp(&other_parent_event.frame_number) {
                Ordering::Equal => {
                    let root_flag_table = strict_merge_flag_table(
                        &self_parent_ft,
                        &other_parent_ft,
                        self_parent_event.frame_number,
                    );
                    let creator_root_flag_table = {
                        let store = self.store.read().unwrap();
                        store.derive_creator_flag_table(&root_flag_table, self_parent_event.frame_number)
                    };
                    let root_majority = { self.conf.read().unwrap().peers.root_majority() };
                    if creator_root_flag_table.len() >= root_majority {
                        root = true;
                        self_parent_event.frame_number + 1
                    } else {
                        root = false;
                        self_parent_event.frame_number
                    }
                },
                Ordering::Greater => {
                    root = false;
                    self_parent_event.frame_number
                },
                Ordering::Less => {
                    root = true;
                    other_parent_event.frame_number
                },
            };

        debug!("{}: * got frame number", self.me_a());
        event.frame_number = frame;
        let first_not_finalised_frame = match self.last_finalised_frame {
            Some(x) => x + 1,
            None => 0,
        };
        let mut visibilis_flag_table =
            open_merge_flag_table(&self_parent_ft, &other_parent_ft, first_not_finalised_frame);
        debug!("{}: * got visibilis ft", self.me_a());
        if root {
            visibilis_flag_table.insert(event_hash.clone(), frame);
        }
        {
            self.store
                .write()
                .unwrap()
                .set_flag_table(&event_hash, &visibilis_flag_table)?;
        }
        debug!("{}: * sign event", self.me_a());
        {
            let cfg = self.conf.read().unwrap();
            let signature = Sig::sign(event_hash, cfg.get_public_key(), cfg.get_secret_key())?;
            event
                .signatures
                .insert(cfg.peers.get_creator_id(), signature);
        }
        debug!("{}: * insert event: {}", self.me_a(), event.clone());

        {
            self.store.write().unwrap().set_event(event)?;
        }
        let creator_visibilis_flag_table = {
            let store = self.store.read().unwrap();
            store.derive_creator_flag_table(&visibilis_flag_table, first_not_finalised_frame + 1)
        };
        let peer_size = { self.conf.read().unwrap().peers.len() };
        debug!(
            "{}: * peer_size: {}; visibilis_ft_size:{}",
            self.me_a(),
            peer_size,
            creator_visibilis_flag_table.len()
        );
        debug!(
            "{}: * ft:{}; creator_ft:{}",
            self.me_a(),
            flag_table_fmt(&visibilis_flag_table),
            creator_flag_table_fmt(&creator_visibilis_flag_table)
        );
        if peer_size == creator_visibilis_flag_table.len() {
            let frame_upto = min_frame(&creator_visibilis_flag_table);
            debug!(
                "{}: first not finalised frame:{}; frame up to: {}",
                self.me_a(),
                first_not_finalised_frame,
                frame_upto
            );
            for frame in first_not_finalised_frame..frame_upto {
                //self.finalise_frame(frame)
                {
                    let mut store = self.store.write().unwrap();
                    let mut frame_itself = store.get_frame(frame)?;
                    frame_itself.finalise();
                    debug!(
                        "{}: +finalised frame {}: {}",
                        self.me_a(),
                        frame,
                        frame_itself.clone()
                    );
                    store.set_frame(frame, frame_itself)?;
                }
                self.last_finalised_frame = Some(frame);
                // notify consumer on next transaction in consensus availability
                if let Some(waker) = { self.conf.write().unwrap().waker.take() } {
                    debug!("{}: calling waker", self.me_a());
                    waker.wake();
                }
            }
        }
        Ok(true)
    }
}
