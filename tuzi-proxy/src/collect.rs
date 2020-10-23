use crate::Protocol;
use async_trait::async_trait;
use chrono::{DateTime, Local};
use indexmap::map::IndexMap;
use nom::lib::std::{collections::LinkedList, fmt::Formatter};
use std::{
    collections::HashMap,
    fmt,
    fmt::{Debug, Display},
    net::SocketAddr,
    time::{Duration, SystemTime},
};
use serde::{Serialize, Serializer};

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
        println!("{}{}", self.prefix.as_ref(), serde_json::to_string(&record).unwrap());
    }
}

#[derive(Debug, Serialize)]
pub struct Record {
    pub protocol: Protocol,
    pub success: bool,
    #[serde(serialize_with = "serialize_system_time")]
    pub start_time: SystemTime,
    #[serde(serialize_with = "serialize_system_time")]
    pub end_time: SystemTime,
    pub client_addr: SocketAddr,
    pub server_addr: SocketAddr,
    pub endpoint: String,
    pub request: Option<HashMap<String, String>>,
    pub response: Option<HashMap<String, String>>,
}

fn serialize_system_time<S>(time: &SystemTime, s: S) -> Result<S::Ok, S::Error> where S: Serializer {
    let datetime: DateTime<Local> = time.clone().into();
    s.serialize_str(&format!("{}", datetime))
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
