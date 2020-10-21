use crate::Protocol;
use indexmap::map::IndexMap;
use nom::lib::std::collections::LinkedList;
use std::{collections::HashMap, net::SocketAddr, time::Duration};

pub trait Collectable {
    fn collect(&mut self, record: Record);
}

#[derive(Debug)]
pub struct Record {
    protocol: Protocol,
    success: bool,
    spend: Duration,
    local: SocketAddr,
    peer: SocketAddr,
    endpoint: String,
    request: Option<HashMap<String, String>>,
    response: Option<HashMap<String, String>>,
}

#[derive(Default)]
pub struct Collection {
    inbounds: LinkedList<Record>,
    outbounds: LinkedList<Record>,
}

impl Collection {
    pub fn insert_inbound(&mut self, record: Record) {
        self.inbounds.push_back(record);
    }

    pub fn insert_outbound(&mut self, record: Record) {
        self.outbounds.push_back(record);
    }
}
