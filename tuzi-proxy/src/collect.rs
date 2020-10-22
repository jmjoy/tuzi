use crate::Protocol;
use async_trait::async_trait;
use indexmap::map::IndexMap;
use nom::lib::std::collections::LinkedList;
use std::{
    collections::HashMap,
    net::SocketAddr,
    time::{Duration, SystemTime},
};

#[async_trait]
pub trait Collectable: Send + Sync {
    async fn collect(&self, record: Record);
}

pub struct StdoutCollector<S: AsRef<str>> {
    prefix: S,
}

impl<S: AsRef<str>> StdoutCollector<S> {
    pub fn new(prefix: S) -> Self {
        Self { prefix }
    }
}

#[async_trait]
impl<S: AsRef<str> + Send + Sync> Collectable for StdoutCollector<S> {
    async fn collect(&self, record: Record) {
        println!("{}{:?}", self.prefix.as_ref(), record);
    }
}

#[derive(Debug)]
pub struct Record {
    pub protocol: Protocol,
    pub success: bool,
    pub start_time: SystemTime,
    pub end_time: SystemTime,
    pub client_addr: SocketAddr,
    pub server_addr: SocketAddr,
    pub endpoint: String,
    pub request: Option<HashMap<String, String>>,
    pub response: Option<HashMap<String, String>>,
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
