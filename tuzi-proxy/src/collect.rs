use crate::Protocol;
use indexmap::map::IndexMap;
use nom::lib::std::collections::LinkedList;
use std::{collections::HashMap, net::SocketAddr, time::Duration};
use async_trait::async_trait;

#[async_trait]
pub trait Collectable: Send + Sync {
    async fn collect(&mut self, record: Record);
}

pub struct StdoutCollector;

#[async_trait]
impl Collectable for StdoutCollector {
    async fn collect(&mut self, record: Record) {
        println!("{:?}", record);
    }
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
